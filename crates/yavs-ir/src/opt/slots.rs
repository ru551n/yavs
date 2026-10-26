//! Constant-size temporaries of a process in its frame (#588, #668).
//!
//! `Alloc` of a constant size is temporary memory: a bump allocation that zeroes, freed by the
//! next `Stmt`. In a process, when every use of the block is in the basic block of the `Alloc`
//! (no pointer to it is live at the end of that block, stored anywhere, returned
//! or rooted) it can be bytes of the process's frame instead: `AddrOf` of the frame, a fresh
//! register defined once (the frame starts with it), and so is every address derived from it by
//! a constant. Blocks of one basic block that are not alive together share bytes. The zeroing
//! stays (`Inst::Zero`) unless the stores, the `MakeDope` or the `Copy` that follow write every
//! word before anything reads it; a store of zero into a word still zero goes. The statements
//! that allocate nothing any more lose their markers (`drop_markers`). Runs last, after inlining
//! and `sink` (which work on `Alloc`s); the predecoder and the LLVM tier have frame forms of
//! the fused `Alloc` ops (`docs/decisions.md` "Temporaries of a process in its frame").
//! Results of calls with a temporary as an argument are taken to alias it.

use super::*;

/// The most bytes of one temporary.
const MAX_TEMP: u32 = 256;
/// The most bytes of slots one process takes.
const MAX_BYTES: u32 = 4 * 1024;
/// The temporaries of one block tracked at a time (a bit each).
const MAX_PER_BLOCK: usize = 64;

struct Cand {
    pc: usize,
    block: usize,
    dst: Reg,
    /// Bytes, a multiple of 8.
    size: u32,
    /// The last instruction that names the block (or a pointer into it).
    end: usize,
    ok: bool,
    /// A handle was stored into it (S2: lowering roots such a value with `TempRoot`, which
    /// rejects the block, so a block taken never holds one).
    handle: bool,
    /// Still only stores to known words so far.
    fresh: bool,
    covered: u64,
    /// Words known to be zero (the `Zero` is there).
    zero: u64,
    /// The block was completely written by stores before anything read it: no `Zero`.
    skip_zero: bool,
    /// Stores of zero into a word that was still zero.
    zero_stores: Vec<usize>,
    at: u32,
}

fn full(size: u32) -> u64 {
    let w = size / 8;
    if w >= 64 { !0 } else { (1u64 << w) - 1 }
}

/// The operands of `i` that may be a pointer to a temporary without the pointer escaping, and
/// the register that then points into the same block (`derive`). The value a `Store` writes is
/// not one of them: a pointer stored anywhere, even into another temporary, keeps its block an
/// `Alloc` (a pointer loaded back out of memory is not tracked, S1 of the #668 review).
struct Use {
    ok: Vec<Reg>,
    derive: Option<Reg>,
    /// Does the instruction only derive an address (no access to the block)?
    pure_derive: bool,
}

fn classify(f: &crate::Func, i: &Inst) -> Option<Use> {
    let mut u = Use {
        ok: Vec::new(),
        derive: None,
        pure_derive: false,
    };
    match *i {
        Inst::Load { addr, .. } => u.ok.push(addr),
        Inst::Store { addr, .. } => u.ok.push(addr),
        Inst::Copy { dst, src, .. } => u.ok.extend([dst, src]),
        Inst::Fill { dst, elem, .. } => u.ok.extend([dst, elem]),
        Inst::Zero { addr, .. } => u.ok.push(addr),
        Inst::Pos { dope, .. } | Inst::SlicePos { dope, .. } | Inst::DopeLen { dope, .. } => {
            u.ok.push(dope)
        }
        Inst::LayoutOf { desc, .. } => u.ok.push(desc),
        Inst::SubtypeCheck { dope, want, .. } => u.ok.extend([dope, want]),
        Inst::MakeDope { at, .. } => u.ok.push(at),
        Inst::Index { dst, base, .. } | Inst::Field { dst, base, .. } => {
            u.ok.push(base);
            u.derive = Some(dst);
            u.pure_derive = true;
        }
        Inst::Move { dst, src } => {
            u.ok.push(src);
            u.derive = Some(dst);
            u.pure_derive = true;
        }
        Inst::LogicOp { dst, a, b, .. } | Inst::LNum { dst, a, b, .. } => u.ok.extend([dst, a, b]),
        Inst::LEq { a, b, .. } | Inst::MemEq { a, b, .. } => u.ok.extend([a, b]),
        Inst::LGet { base, .. } | Inst::LSet { base, .. } | Inst::LFill { base, .. } => {
            u.ok.push(base)
        }
        Inst::LCopy { dst, src, .. } => u.ok.extend([dst, src]),
        Inst::Cat { dst, parts, .. } => {
            u.ok.push(dst);
            u.ok.extend_from_slice(f.regs(parts)?);
        }
        Inst::SigRead { dst, .. } | Inst::SigLastValue { dst, .. } => u.ok.push(dst),
        Inst::DrivingValue { dst, .. } => u.ok.push(dst),
        Inst::Drive { wave, .. } | Inst::DriveSlot { wave, .. } => u.ok.push(wave),
        Inst::Force { value, .. } => u.ok.push(value),
        Inst::Report { msg, .. } => u.ok.push(msg),
        Inst::Output { addr, .. } => u.ok.push(addr),
        Inst::New { init, .. } => u.ok.push(init),
        Inst::NewFill { desc, data, .. } | Inst::NewFrame { desc, data, .. } => {
            u.ok.extend([desc, data])
        }
        Inst::Call { args, dst, .. }
        | Inst::CallNative { args, dst, .. }
        | Inst::CallForeign { args, dst, .. } => {
            u.ok.extend_from_slice(f.regs(args)?);
            u.derive = dst;
        }
        Inst::CallIndirect { args, dst, .. } | Inst::CallProt { args, dst, .. } => {
            u.ok.extend_from_slice(f.regs(args)?);
            u.derive = dst;
        }
        _ => {}
    }
    Some(u)
}

fn operands(f: &crate::Func, i: &Inst, out: &mut Vec<Reg>, defs: &mut Vec<Reg>) {
    i.visit(&mut |r, role, _| match role {
        Role::Use => out.push(r),
        Role::Def => defs.push(r),
    });
    // A `MakeDope`/`Cat` list is read; a call's argument list too.
    if let Some(l) = i.list()
        && let Some(rs) = f.regs(l)
    {
        out.extend_from_slice(rs);
    }
}

/// Take the temporaries of process `f` that can live in its frame. Returns whether the code
/// changed (the caller then drops the `Stmt` markers left with nothing to free).
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    if f.kind != FuncKind::Process || f.nested {
        return false;
    }
    let nreg = f.reg_ty.len();
    let mut defs = vec![0u32; nreg];
    let mut konst: FxMap<Reg, i64> = FxMap::default();
    {
        let (mut us, mut ds) = (Vec::new(), Vec::new());
        for i in &f.code {
            us.clear();
            ds.clear();
            operands(f, i, &mut us, &mut ds);
            for r in &ds {
                if let Some(x) = defs.get_mut(r.0 as usize) {
                    *x += 1;
                }
            }
            if let Inst::Const { dst, v } = *i {
                konst.insert(dst, v);
            }
        }
    }
    konst.retain(|r, _| defs.get(r.0 as usize) == Some(&1));

    let mut cands: Vec<Cand> = Vec::new();
    // The registers that still point into temporaries at the end of their block.
    let mut out: Vec<(usize, Reg, Vec<usize>)> = Vec::new();
    for b in 0..f.blocks.len() {
        for (r, who) in scan_block(f, b, &konst, &mut cands) {
            out.push((b, r, who));
        }
    }
    if cands.is_empty() {
        return false;
    }
    // A pointer some later block may read: that temporary lives on.
    let mut tracked: Vec<Reg> = out.iter().map(|o| o.1).collect();
    tracked.sort_unstable_by_key(|r| r.0);
    tracked.dedup();
    let Some(live) = live_out(prog, f, &tracked) else {
        return false;
    };
    for (b, r, who) in out {
        let Ok(k) = tracked.binary_search_by_key(&r.0, |t| t.0) else {
            continue;
        };
        if live[b][k / 64] >> (k % 64) & 1 == 1 {
            for c in who {
                cands[c].ok = false;
            }
        }
    }
    // First fit among the temporaries of a block that are alive together.
    let mut peak = 0u32;
    let mut order: Vec<usize> = (0..cands.len()).filter(|&c| cands[c].ok).collect();
    order.sort_by_key(|&c| (cands[c].block, cands[c].pc));
    let mut placed: Vec<usize> = Vec::new();
    for &c in &order {
        if placed
            .last()
            .is_some_and(|&p| cands[p].block != cands[c].block)
        {
            placed.clear();
        }
        let (pc, size) = (cands[c].pc, cands[c].size);
        let mut at = 0u32;
        'fit: loop {
            for &p in &placed {
                let q = &cands[p];
                if q.end >= pc && at < q.at + q.size && q.at < at + size {
                    at = (q.at + q.size).next_multiple_of(8);
                    continue 'fit;
                }
            }
            break;
        }
        if at + size > MAX_BYTES {
            cands[c].ok = false;
            continue;
        }
        cands[c].at = at;
        peak = peak.max(at + size);
        placed.push(c);
    }
    let taken: Vec<usize> = (0..cands.len()).filter(|&c| cands[c].ok).collect();
    if taken.is_empty() {
        return false;
    }
    debug_assert!(
        taken.iter().all(|&c| !cands[c].handle),
        "a temporary holding a handle in the frame of {} (S2: it needs its `TempRoot`)",
        f.name
    );
    // Fresh registers for each block and the addresses derived from it, up to its last use: the
    // frame starts with them (defined once), whatever register sharing made of the `Alloc`'s.
    for &c in &taken {
        let fresh = Reg(f.reg_ty.len() as u32);
        f.reg_ty.push(RegTy::Ptr);
        let mut map: FxMap<Reg, Reg> = FxMap::default();
        map.insert(cands[c].dst, fresh);
        cands[c].dst = fresh;
        for pc in cands[c].pc + 1..=cands[c].end {
            rename(f, pc, &mut map);
        }
    }
    let base = f.frame.next_multiple_of(8);
    let mut at_pc: FxMap<usize, usize> = FxMap::default();
    let mut skip: FxMap<usize, ()> = FxMap::default();
    for &c in &taken {
        at_pc.insert(cands[c].pc, c);
        if !cands[c].skip_zero {
            skip.extend(cands[c].zero_stores.iter().map(|&p| (p, ())));
        }
    }
    let mut code = Vec::with_capacity(f.code.len() + taken.len());
    let mut loc = Vec::with_capacity(code.capacity());
    // (Per instruction, the calls it was inlined through: empty when nothing was inlined.)
    let mut via = Vec::with_capacity(if f.via.is_empty() { 0 } else { code.capacity() });
    let mut ranges = Vec::with_capacity(f.blocks.len());
    // The frame offsets the (fresh) registers of the blocks hold: an address derived from one by
    // a constant is a constant too, which the frame starts with.
    let mut addr: FxMap<Reg, u32> = FxMap::default();
    let fold = |i: Inst, addr: &FxMap<Reg, u32>| -> Option<(Reg, u32)> {
        match i {
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let o = i64::from(*addr.get(&base)?)
                    .checked_add(konst.get(&pos)?.checked_mul(i64::from(stride))?)?;
                Some((dst, u32::try_from(o).ok()?))
            }
            Inst::Move { dst, src } => Some((dst, *addr.get(&src)?)),
            _ => None,
        }
    };
    for b in f.blocks.values() {
        let start = code.len() as u32;
        for pc in b.start as usize..b.end as usize {
            if skip.contains_key(&pc) {
                continue;
            }
            if let Some(&v) = f.via.get(pc) {
                via.push(v);
            }
            let Some(&c) = at_pc.get(&pc) else {
                let i = match fold(f.code[pc], &addr) {
                    Some((dst, off)) => {
                        addr.insert(dst, off);
                        Inst::AddrOf {
                            dst,
                            space: AddrSpace::Frame,
                            off,
                        }
                    }
                    None => f.code[pc],
                };
                code.push(i);
                loc.push(f.loc[pc]);
                continue;
            };
            let k = &cands[c];
            addr.insert(k.dst, base + k.at);
            code.push(Inst::AddrOf {
                dst: k.dst,
                space: AddrSpace::Frame,
                off: base + k.at,
            });
            loc.push(f.loc[pc]);
            if !k.skip_zero && k.size > 0 {
                code.push(Inst::Zero {
                    addr: k.dst,
                    bytes: k.size,
                });
                loc.push(f.loc[pc]);
                if let Some(&v) = f.via.get(pc) {
                    via.push(v);
                }
            }
        }
        ranges.push((start, code.len() as u32));
    }
    for (i, (s, e)) in ranges.into_iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(crate::BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
    f.frame = base + peak;
    true
}

/// Rename the registers of `map` that the instruction at `pc` reads; a derived address (an
/// `Index`, `Field` or `Move` of a renamed register) gets a fresh register itself, and a register
/// it writes otherwise is no longer renamed after it.
fn rename(f: &mut crate::Func, pc: usize, map: &mut FxMap<Reg, Reg>) {
    let mut i = f.code[pc];
    i.visit_mut(&mut |r, role, _| {
        if role == Role::Use
            && let Some(&n) = map.get(r)
        {
            *r = n;
        }
    });
    if let Some(l) = i.list()
        && let Some(rs) = f.regs(l)
        && rs.iter().any(|r| map.contains_key(r))
    {
        // A list may be shared with a copy of the instruction: then rename a copy of it (the
        // predecoder counts every operand, so no list is left without its instruction).
        let new: Vec<Reg> = rs
            .iter()
            .map(|r| map.get(r).copied().unwrap_or(*r))
            .collect();
        let shared = f.code.iter().filter(|j| j.list() == Some(l)).count() > 1;
        if shared {
            let start = f.operands.len() as u32;
            f.operands.extend(new);
            if let Some(lm) = i.list_mut() {
                *lm = crate::Regs { start, len: l.len };
            }
        } else {
            let at = l.start as usize;
            f.operands[at..at + new.len()].copy_from_slice(&new);
        }
    }
    let renamed = |r: Reg| map.values().any(|&n| n == r);
    let derived = match i {
        Inst::Index { dst, base, .. } | Inst::Field { dst, base, .. } if renamed(base) => Some(dst),
        Inst::Move { dst, src } if renamed(src) => Some(dst),
        _ => None,
    };
    let mut defs = Vec::new();
    i.visit(&mut |r, role, _| {
        if role == Role::Def {
            defs.push(r);
        }
    });
    for d in defs {
        map.remove(&d);
    }
    if let Some(d) = derived {
        let n = Reg(f.reg_ty.len() as u32);
        f.reg_ty
            .push(f.reg_ty.get(d.0 as usize).copied().unwrap_or(RegTy::Ptr));
        i.visit_mut(&mut |r, role, _| {
            if role == Role::Def && *r == d {
                *r = n;
            }
        });
        map.insert(d, n);
    }
    f.code[pc] = i;
}

/// An `Alloc` of 32 bytes that the interpreter fuses with the read of a signal, the copy or the
/// concatenation into it runs in its dispatch loop; the frame forms of those fusions are out of
/// line and cost the designs that run them every cycle 2 to 8 percent in cycles (#756). Only
/// the interpreter alone keeps those blocks (compiled code is better with them in the frame).
fn fused_in_loop(f: &crate::Func, pc: usize, end: usize, dst: Reg) -> bool {
    // The constants and addresses in between are skipped by the predecoder.
    let next = f.code[pc + 1..end].iter().find(|i| {
        !matches!(
            i,
            Inst::Const { .. } | Inst::ConstF { .. } | Inst::AddrOf { .. }
        )
    });
    matches!(
        next,
        Some(
            &Inst::SigRead { dst: d, .. }
                | &Inst::LCopy { dst: d, .. }
                | &Inst::Cat {
                    dst: d,
                    logic: true,
                    ..
                }
        ) if d == dst
    )
}

/// Find the temporaries of block `b` (the `Alloc`s of constant size) and what the block's
/// instructions do with their pointers.
fn scan_block(
    f: &crate::Func,
    b: usize,
    konst: &FxMap<Reg, i64>,
    cands: &mut Vec<Cand>,
) -> Vec<(Reg, Vec<usize>)> {
    let Some(blk) = f.blocks.get(crate::BlockId(b as u32)) else {
        return Vec::new();
    };
    // Registers that point into a temporary: a bit per temporary of this block; the word
    // offset when it is one temporary's and known.
    let mut owner: FxMap<Reg, u64> = FxMap::default();
    let mut off: FxMap<Reg, u32> = FxMap::default();
    let mut local: Vec<usize> = Vec::new();
    let mut cv: FxMap<Reg, i64> = FxMap::default();
    let (mut us, mut ds) = (Vec::new(), Vec::new());
    let konst_of = |cv: &FxMap<Reg, i64>, r: Reg| cv.get(&r).or_else(|| konst.get(&r)).copied();
    for pc in blk.start as usize..blk.end as usize {
        let i = f.code[pc];
        us.clear();
        ds.clear();
        operands(f, &i, &mut us, &mut ds);
        // Owners of the registers read.
        let mut touched = 0u64;
        let mut seen: Vec<(Reg, u64)> = Vec::new();
        for r in &us {
            if let Some(&m) = owner.get(r) {
                touched |= m;
                seen.push((*r, m));
            }
        }
        let mut derived = 0u64;
        if touched != 0 {
            let bits = |m: u64| (0..64).filter(move |k| m >> k & 1 == 1);
            let all_of = |m: u64, local: &Vec<usize>| bits(m).map(|k| local[k]).collect::<Vec<_>>();
            let reject = |m: u64, cands: &mut Vec<Cand>, local: &Vec<usize>| {
                for g in all_of(m, local) {
                    cands[g].ok = false;
                }
            };
            match classify(f, &i) {
                None => reject(touched, cands, &local),
                Some(u) => {
                    for (r, m) in &seen {
                        if !u.ok.contains(r) {
                            // A pointer used some other way (stored, returned, compared).
                            reject(*m, cands, &local);
                        }
                    }
                    if u.derive.is_some() {
                        derived = touched;
                    }
                    // The effect on each block touched, for the zeroing.
                    for g in all_of(touched, &local) {
                        let c = &mut cands[g];
                        c.end = pc;
                        let own = |m: Option<&u64>| m == Some(&(1u64 << local_bit(&local, g)));
                        match i {
                            Inst::Store { addr, src, w } if own(owner.get(&addr)) => {
                                c.handle |= f.reg_ty.get(src.0 as usize) == Some(&RegTy::Handle);
                                match off.get(&addr).copied() {
                                    Some(o) if w == Width::B8 && o % 8 == 0 && o + 8 <= c.size => {
                                        let word = o / 8;
                                        if c.fresh {
                                            c.covered |= 1u64 << word;
                                        }
                                        let is_zero = konst_of(&cv, src) == Some(0);
                                        if is_zero && c.zero >> word & 1 == 1 {
                                            c.zero_stores.push(pc);
                                        } else {
                                            c.zero &= !(1u64 << word);
                                        }
                                    }
                                    _ => end_fresh(c),
                                }
                            }
                            // Every word written at once from the start of the block.
                            Inst::MakeDope { at, parts }
                                if own(owner.get(&at))
                                    && off.get(&at) == Some(&0)
                                    && 8 * parts.len >= c.size =>
                            {
                                if c.fresh {
                                    c.covered = full(c.size);
                                }
                                end_fresh(c);
                            }
                            Inst::Copy { dst, src, bytes }
                                if own(owner.get(&dst))
                                    && !owner.contains_key(&src)
                                    && off.get(&dst) == Some(&0)
                                    && konst_of(&cv, bytes)
                                        .is_some_and(|n| n >= i64::from(c.size)) =>
                            {
                                if c.fresh {
                                    c.covered = full(c.size);
                                }
                                end_fresh(c);
                            }
                            _ if u.pure_derive => {}
                            _ => end_fresh(c),
                        }
                    }
                }
            }
        }
        // Definitions end what a register pointed to.
        for r in &ds {
            owner.remove(r);
            off.remove(r);
            cv.remove(r);
        }
        if let Inst::Const { dst, v } = i {
            cv.insert(dst, v);
        }
        if derived != 0 {
            let target = classify(f, &i).and_then(|u| u.derive);
            if let Some(d) = target {
                owner.insert(d, derived);
                match i {
                    Inst::Index {
                        base, pos, stride, ..
                    } => {
                        if let (Some(&o), Some(p)) = (off.get(&base), konst_of(&cv, pos))
                            && derived.count_ones() == 1
                            && let Some(x) = p.checked_mul(i64::from(stride))
                            && let Ok(x) = u32::try_from(x)
                        {
                            off.insert(d, o.saturating_add(x));
                        }
                    }
                    Inst::Move { src, .. } => {
                        if let Some(&o) = off.get(&src) {
                            off.insert(d, o);
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Inst::Alloc { dst, size } = i
            && let Some(n) = konst_of(&cv, size)
            && let Ok(n) = u32::try_from(n)
            && n > 0
            && n <= MAX_TEMP
            && local.len() < MAX_PER_BLOCK
            && !(n == 32 && !frame_blocks_on() && fused_in_loop(f, pc, blk.end as usize, dst))
        {
            let n = n.next_multiple_of(8);
            let g = cands.len();
            cands.push(Cand {
                pc,
                block: b,
                dst,
                size: n,
                end: pc,
                ok: true,
                handle: false,
                fresh: true,
                covered: 0,
                zero: full(n),
                skip_zero: false,
                zero_stores: Vec::new(),
                at: 0,
            });
            owner.insert(dst, 1u64 << local.len());
            off.insert(dst, 0);
            local.push(g);
        }
    }
    // Blocks that stayed fresh to the end of the block.
    for &g in &local {
        let c = &mut cands[g];
        if c.fresh {
            end_fresh(c);
        }
    }
    owner
        .into_iter()
        .map(|(r, m)| {
            (
                r,
                (0..local.len())
                    .filter(|k| m >> k & 1 == 1)
                    .map(|k| local[k])
                    .collect(),
            )
        })
        .collect()
}

/// Which of `tracked` (sorted) may be read after the end of each block before they are written:
/// a bit set per block, bit `k` for `tracked[k]`. `None` for a function too big for the sets
/// (ponytail: a process of tens of thousands of blocks and pointers keeps its `Alloc`s; a sparse
/// set would lift the cap).
fn live_out(prog: &Program, f: &crate::Func, tracked: &[Reg]) -> Option<Vec<Vec<u64>>> {
    let nb = f.blocks.len();
    let w = tracked.len().div_ceil(64).max(1);
    if nb.checked_mul(w)? > 1 << 24 {
        return None;
    }
    let bit = |r: &Reg| tracked.binary_search_by_key(&r.0, |t| t.0).ok();
    let (mut gen_, mut kill) = (vec![vec![0u64; w]; nb], vec![vec![0u64; w]; nb]);
    let (mut us, mut ds) = (Vec::new(), Vec::new());
    for (b, blk) in f.blocks.values().iter().enumerate() {
        for i in &f.code[blk.start as usize..blk.end as usize] {
            us.clear();
            ds.clear();
            operands(f, i, &mut us, &mut ds);
            for k in us.iter().filter_map(bit) {
                if kill[b][k / 64] >> (k % 64) & 1 == 0 {
                    gen_[b][k / 64] |= 1 << (k % 64);
                }
            }
            for k in ds.iter().filter_map(bit) {
                kill[b][k / 64] |= 1 << (k % 64);
            }
        }
    }
    let succ = super::memo::successors(prog, f);
    let (mut live_in, mut live) = (vec![vec![0u64; w]; nb], vec![vec![0u64; w]; nb]);
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut o = vec![0u64; w];
            for &s in &succ[b] {
                match live_in.get(s) {
                    Some(x) => o.iter_mut().zip(x).for_each(|(o, y)| *o |= y),
                    // (A successor that is no block: everything is taken as read there.)
                    None => o.fill(!0),
                }
            }
            let inn: Vec<u64> = (0..w).map(|k| gen_[b][k] | (o[k] & !kill[b][k])).collect();
            if inn != live_in[b] || o != live[b] {
                live_in[b] = inn;
                live[b] = o;
                changed = true;
            }
        }
    }
    Some(live)
}

fn local_bit(local: &[usize], g: usize) -> usize {
    local.iter().position(|&x| x == g).unwrap_or(0)
}

/// Something other than a store to a known word touches the block: from here on it is read, so
/// it is whole or it is zeroed first.
fn end_fresh(c: &mut Cand) {
    if c.fresh {
        c.fresh = false;
        c.skip_zero = c.covered == full(c.size);
    }
    c.zero = 0;
}
