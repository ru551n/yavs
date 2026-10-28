//! Which registers of a frame-mode function a leaving path needs in the frame, and which frame
//! variables compiled code keeps as values (#432).
//!
//! Compiled code runs on the interpreter's register file, so every definition used to be a store.
//! Most of them are temporaries nobody reads from the frame: the interpreter reads a register
//! only where compiled code leaves it (a guard failing, a stepped instruction, a block edge that
//! the interpreter may enter). `Live` gives the registers live before each instruction, and
//! `Gen::flush` stores a definition only where it is live and the frame is looked at. `preloaded`
//! names the registers the interpreter starts every frame with (defined once by a constant or an
//! address): compiled code never stores those.

use yavs_ir::{AddrSpace, Func, Inst, Program, Reg, RegTy, Role, Width};

/// Registers live before each instruction (`bits[pc * words ..]`), `n + 1` rows (the last is empty).
pub struct Live {
    words: usize,
    bits: Vec<u64>,
}

/// Functions whose rows would take more than this many words get no deferral.
const MAX_WORDS: usize = 1 << 25;

impl Live {
    pub fn row(&self, pc: usize) -> &[u64] {
        &self.bits[pc * self.words..(pc + 1) * self.words]
    }

    /// Per register (and slot variable), the number of instructions it is live before.
    pub fn spans(&self) -> Vec<u32> {
        let mut out = vec![0u32; self.words * 64];
        for row in self.bits.chunks(self.words) {
            for (w, &bits) in row.iter().enumerate() {
                let mut b = bits;
                while b != 0 {
                    out[w * 64 + b.trailing_zeros() as usize] += 1;
                    b &= b - 1;
                }
            }
        }
        out
    }

    /// Backward liveness over the instructions of `f`; `None` for a function too big for it.
    ///
    /// Solved per basic block (a few hundred times fewer sets than instructions), the rows of
    /// the instructions then come from one backward sweep of each block (#710).
    pub fn new(prog: &Program, f: &Func, sc: Option<&Scalars>) -> Option<Live> {
        let n = f.code.len();
        let nv = sc.map_or(0, |s| s.slots.len());
        // One bit past the slot variables is the frame base (`Gen`), never live.
        let words = (f.reg_ty.len() + nv + 1).div_ceil(64).max(1);
        if (n + 1).checked_mul(words)? > MAX_WORDS {
            return None;
        }
        // The bits each instruction uses and defines, and its successors.
        let (mut uses, mut defs): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
        let (mut uat, mut dat) = (Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
        let mut succ: Vec<Vec<u32>> = Vec::with_capacity(n);
        for (pc, i) in f.code.iter().enumerate() {
            uat.push(uses.len());
            dat.push(defs.len());
            let mark = |v: &mut Vec<u32>, r: Reg| {
                // (A register beyond the frame is no bit, as before.)
                if (r.0 as usize) < words * 64 {
                    v.push(r.0);
                }
                if f.reg_ty.get(r.0 as usize) == Some(&RegTy::Sig)
                    && ((r.0 + 1) as usize) < words * 64
                {
                    v.push(r.0 + 1);
                }
            };
            i.visit(&mut |r, role, _| {
                mark(
                    if role == Role::Def {
                        &mut defs
                    } else {
                        &mut uses
                    },
                    r,
                )
            });
            if let Some(l) = i.list() {
                for &r in f.regs(l)? {
                    mark(&mut uses, r);
                }
            }
            if let Some(sc) = sc {
                let bit = |k: usize| (f.reg_ty.len() + k) as u32;
                match (sc.at[pc], i) {
                    (Some(k), Inst::Load { .. }) => uses.push(bit(k as usize)),
                    (Some(k), Inst::Store { .. }) => defs.push(bit(k as usize)),
                    (_, i) if sc.observes(i) => {
                        uses.extend((0..nv).filter(|&k| sc.stored[k]).map(bit));
                    }
                    _ => {}
                }
            }
            let s = succs(prog, f, pc, i)?;
            if s.iter().any(|&t| t as usize >= n) {
                return None;
            }
            succ.push(s);
        }
        uat.push(uses.len());
        dat.push(defs.len());
        let (uses_of, defs_of) = (
            |pc: usize| &uses[uat[pc]..uat[pc + 1]],
            |pc: usize| &defs[dat[pc]..dat[pc + 1]],
        );
        // Basic blocks: a block ends at an instruction that is not followed by its only successor.
        let mut start = vec![false; n + 1];
        start[0] = true;
        for (pc, s) in succ.iter().enumerate() {
            if s.as_slice() != [pc as u32 + 1] {
                start[pc + 1] = true;
                for &t in s {
                    start[t as usize] = true;
                }
            }
        }
        let starts: Vec<usize> = (0..n).filter(|&pc| start[pc]).collect();
        let nb = starts.len();
        let mut block_of = vec![0u32; n + 1];
        for (b, &s) in starts.iter().enumerate() {
            let end = starts.get(b + 1).copied().unwrap_or(n);
            block_of[s..end].fill(b as u32);
        }
        let bsucc: Vec<Vec<u32>> = (0..nb)
            .map(|b| {
                let last = starts.get(b + 1).copied().unwrap_or(n) - 1;
                succ[last].iter().map(|&t| block_of[t as usize]).collect()
            })
            .collect();
        // Upward exposed uses and definitions of each block.
        let (mut gen_, mut kill) = (vec![0u64; nb * words], vec![0u64; nb * words]);
        for b in 0..nb {
            let end = starts.get(b + 1).copied().unwrap_or(n);
            let (g, k) = (
                &mut gen_[b * words..(b + 1) * words],
                &mut kill[b * words..(b + 1) * words],
            );
            for pc in starts[b]..end {
                for &u in uses_of(pc) {
                    if k[u as usize / 64] >> (u % 64) & 1 == 0 {
                        g[u as usize / 64] |= 1 << (u % 64);
                    }
                }
                for &d in defs_of(pc) {
                    k[d as usize / 64] |= 1 << (d % 64);
                }
            }
        }
        let mut live_in = gen_;
        let mut out = vec![0u64; words];
        let mut converged = false;
        for _ in 0..64 {
            let mut changed = false;
            for b in (0..nb).rev() {
                out.fill(0);
                for &s in &bsucc[b] {
                    for (o, x) in out.iter_mut().zip(&live_in[s as usize * words..]) {
                        *o |= x;
                    }
                }
                for (w, o) in out.iter().enumerate() {
                    let at = b * words + w;
                    // `live_in` starts as the block's own uses and only grows.
                    let v = live_in[at] | (o & !kill[at]);
                    if v != live_in[at] {
                        live_in[at] = v;
                        changed = true;
                    }
                }
            }
            if !changed {
                converged = true;
                break;
            }
        }
        if !converged {
            return None;
        }
        // The rows: from the live-out set of each block back through its instructions.
        let mut bits = vec![0u64; (n + 1) * words];
        for b in 0..nb {
            out.fill(0);
            for &s in &bsucc[b] {
                for (o, x) in out.iter_mut().zip(&live_in[s as usize * words..]) {
                    *o |= x;
                }
            }
            let end = starts.get(b + 1).copied().unwrap_or(n);
            for pc in (starts[b]..end).rev() {
                for &d in defs_of(pc) {
                    out[d as usize / 64] &= !(1 << (d % 64));
                }
                for &u in uses_of(pc) {
                    out[u as usize / 64] |= 1 << (u % 64);
                }
                bits[pc * words..(pc + 1) * words].copy_from_slice(&out);
            }
        }
        Some(Live { words, bits })
    }
}

/// The registers the interpreter starts a frame of `f` with a value in: defined exactly once,
/// by a constant or an address that is not an `Up` one (`Pre::new`).
pub fn preloaded(f: &Func) -> Vec<bool> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u8; n];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
    }
    let mut pre = vec![false; n];
    for i in &f.code {
        let dst = match *i {
            Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => dst,
            Inst::AddrOf { dst, space, .. } if !matches!(space, AddrSpace::Up(_)) => dst,
            _ => continue,
        };
        if defs.get(dst.0 as usize) == Some(&1) {
            pre[dst.0 as usize] = true;
        }
    }
    pre
}

/// The fewest instructions after its definition a register must be live at to be a core
/// register (`entry_core`).
const CORE_MIN: usize = 8;

/// The registers a native frame's exits record the same way wherever they are: defined once,
/// by an instruction of the function's first block (which no jump leads back to), live at
/// `CORE_MIN` or more later instructions, and not preloaded. They dominate every exit from the
/// pc `core_from` on, so those exits can record them in one shared block (`Gen::core_stub`)
/// instead of each storing them again.
pub fn entry_core(prog: &Program, f: &Func, live: &Live, pre: &[bool]) -> (Vec<u32>, u32) {
    let n = f.code.len();
    let end = f
        .code
        .iter()
        .position(Inst::is_terminator)
        .map_or(n, |p| p + 1);
    let entered = (0..n).any(|pc| succs(prog, f, pc, &f.code[pc]).is_none_or(|s| s.contains(&0)));
    if entered {
        return (Vec::new(), 0);
    }
    let mut defs = vec![0u8; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
    }
    let sig = |r: usize| f.reg_ty.get(r) == Some(&RegTy::Sig);
    let (mut core, mut from) = (Vec::new(), 0);
    for (pc, i) in f.code[..end].iter().enumerate() {
        i.visit(&mut |r, role, _| {
            let k = r.0 as usize;
            if role == Role::Def
                && defs.get(k) == Some(&1)
                && !pre.get(k).copied().unwrap_or(true)
                && !sig(k)
                && !(k > 0 && sig(k - 1))
                && (pc + 1..n)
                    .filter(|&p| live.row(p)[k / 64] >> (k % 64) & 1 != 0)
                    .count()
                    >= CORE_MIN
            {
                core.push(r.0);
                from = from.max(pc as u32 + 1);
            }
        });
    }
    (core, from)
}

/// The instructions that can run after `i` at `pc` (`None`: a branch to a missing block).
pub fn succs(prog: &Program, f: &Func, pc: usize, i: &Inst) -> Option<Vec<u32>> {
    let start = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start);
    Some(match *i {
        Inst::Br { to } => vec![start(to)?],
        Inst::CondBr { t, f: e, .. } => vec![start(t)?, start(e)?],
        Inst::Case { table, .. } => {
            let t = prog.cases.get(table)?;
            let mut v = vec![start(t.default)?];
            for &(_, _, to) in &t.arms {
                v.push(start(to)?);
            }
            v
        }
        // The generator steps an `Unreachable` and goes on after it, so do the analyses.
        Inst::Ret { .. } => Vec::new(),
        _ if pc + 1 < f.code.len() => vec![pc as u32 + 1],
        _ => Vec::new(),
    })
}

/// What `reach` finds: the instructions reached, the loop headers and the retreating edges.
pub struct Reach {
    pub reached: Vec<bool>,
    pub head: Vec<bool>,
    /// The natural loop of each retreating edge: its instructions.
    pub loops: Vec<Vec<usize>>,
}

/// The instructions reached from instruction 0 and from `extra` entry points, and the targets
/// of retreating edges of a depth-first search from them (loop headers: the edge closes a cycle).
pub fn reach(prog: &Program, f: &Func, extra: &[usize]) -> Option<Reach> {
    let n = f.code.len();
    let mut state = vec![0u8; n]; // 1 on the stack, 2 done
    let mut head = vec![false; n];
    let mut back = Vec::new();
    let mut stack: Vec<(usize, Vec<u32>)> = Vec::new();
    for &root in std::iter::once(&0).chain(extra) {
        if root >= n || state[root] != 0 {
            continue;
        }
        state[root] = 1;
        stack.push((root, succs(prog, f, root, &f.code[root])?));
        while let Some((pc, todo)) = stack.last_mut() {
            let pc = *pc;
            let Some(s) = todo.pop() else {
                state[pc] = 2;
                stack.pop();
                continue;
            };
            let s = s as usize;
            if s >= n {
                return None;
            }
            match state[s] {
                0 => {
                    state[s] = 1;
                    stack.push((s, succs(prog, f, s, &f.code[s])?));
                }
                1 => {
                    head[s] = true;
                    back.push((pc, s));
                }
                _ => {}
            }
        }
    }
    // The natural loop of a retreating edge `u -> h`: `h` and what reaches `u` without it.
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (pc, i) in f.code.iter().enumerate() {
        if state[pc] != 0 {
            for s in succs(prog, f, pc, i)? {
                preds.get_mut(s as usize)?.push(pc as u32);
            }
        }
    }
    let loops = back
        .iter()
        .map(|&(u, h)| {
            let mut body = vec![h];
            let mut seen = vec![false; n];
            seen[h] = true;
            let mut work = vec![u];
            while let Some(x) = work.pop() {
                if std::mem::replace(&mut seen[x], true) {
                    continue;
                }
                body.push(x);
                work.extend(preds[x].iter().map(|&p| p as usize));
            }
            body
        })
        .collect();
    Some(Reach {
        reached: state.iter().map(|&s| s != 0).collect(),
        head,
        loops,
    })
}

/// Frame variables a function reads and writes only through their own address registers, at
/// one width: compiled code keeps their values in SSA registers like those of the registers
/// (`Gen`: virtual register `nregs + k` is slot `k`), and writes them back where something can
/// look at the frame.
pub struct Scalars {
    /// The slot an instruction (a `Load` or `Store`) reads or writes.
    pub at: Vec<Option<u32>>,
    /// Offset and width of each slot.
    pub slots: Vec<(u32, Width)>,
    /// Whether the function writes the slot (else the value is only a copy of the frame's).
    pub stored: Vec<bool>,
}

/// Slots kept as values: the most that one function gets.
const MAX_SLOTS: usize = 12;

/// Instructions that may collect garbage, run other code or end the activation: what they do
/// before returning to compiled code can look at the frame, so slot variables are written
/// back before them (and a loop with one gains little from keeping variables as values).
pub fn loud(i: &Inst, native: &dyn Fn(&Inst) -> bool) -> bool {
    // A call the compiled code makes directly to a native frame runs no step: it cannot collect.
    if let Inst::Call { .. } = i {
        return !native(i);
    }
    matches!(
        i,
        Inst::CallNative { .. }
            | Inst::CallForeign { .. }
            | Inst::CallIndirect { .. }
            | Inst::CallProt { .. }
            | Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::NewFrame { .. }
            | Inst::Free { .. }
            | Inst::PkgAlloc { .. }
            | Inst::Report { .. }
            | Inst::Output { .. }
            | Inst::Suspend { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitEdge { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::Elab { .. }
            | Inst::Unreachable { .. }
    )
}

impl Scalars {
    /// Whether the interpreter or a callback may look at the frame in the instruction `i`
    /// (it can leave to the interpreter or calls out); the ones that cannot do neither are
    /// listed.
    pub fn observes(&self, i: &Inst) -> bool {
        !matches!(
            i,
            Inst::Const { .. }
                | Inst::ConstF { .. }
                | Inst::Move { .. }
                | Inst::Select { .. }
                | Inst::Cover { .. }
                | Inst::CoverIn { .. }
                | Inst::Cmp { .. }
                | Inst::FCmp { .. }
                | Inst::Bool { .. }
                | Inst::FUn { .. }
                | Inst::IToF { .. }
                | Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Case { .. }
                | Inst::Ret { .. }
        ) && !matches!(i, Inst::AddrOf { space, .. } if !matches!(space, AddrSpace::Up(_)))
    }

    /// The slots of `f` that are accessed in a loop; `None` when there are none or `f` can have
    /// its frame reached from another function (`Func::nested`).
    pub fn new(f: &Func, r: &Reach, native: &dyn Fn(&Inst) -> bool) -> Option<Scalars> {
        let n = f.code.len();
        let nregs = f.reg_ty.len();
        if f.nested {
            return None;
        }
        let mut in_loop = vec![false; n];
        for body in &r.loops {
            // A loop that calls out or waits writes the variables back each round: not worth it.
            if body.iter().any(|&x| loud(&f.code[x], native)) {
                continue;
            }
            for &x in body {
                in_loop[x] = true;
            }
        }
        // Uses of each register, and the ones that are the address of a load or store.
        let mut uses = vec![0u32; nregs];
        let mut as_addr = vec![0u32; nregs];
        let mut defs = vec![0u32; nregs];
        let mut width: Vec<Option<Width>> = vec![None; nregs];
        let mut mixed = vec![false; nregs];
        for i in &f.code {
            i.visit(&mut |r, role, _| {
                if let Some(x) = if role == Role::Def {
                    &mut defs
                } else {
                    &mut uses
                }
                .get_mut(r.0 as usize)
                {
                    *x += 1;
                }
            });
            if let Some(l) = i.list() {
                for &r in f.regs(l)? {
                    *uses.get_mut(r.0 as usize)? += 1;
                }
            }
            if let Inst::Load { addr, w, .. } | Inst::Store { addr, w, .. } = *i {
                let a = addr.0 as usize;
                if w == Width::Sig {
                    mixed[a] = true;
                } else {
                    as_addr[a] += 1;
                    if *width[a].get_or_insert(w) != w {
                        mixed[a] = true;
                    }
                }
            }
        }
        // Address registers of a frame variable: defined once, by `AddrOf`, used only as addresses.
        let pre = preloaded(f);
        let mut cand: Vec<(u32, Reg, Width)> = Vec::new();
        let mut escaped: Vec<u32> = Vec::new();
        for i in &f.code {
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } = *i
            {
                let d = dst.0 as usize;
                match width[d] {
                    Some(w) if pre[d] && !mixed[d] && uses[d] == as_addr[d] => {
                        cand.push((off, dst, w))
                    }
                    _ if pre[d] && uses[d] == 0 && defs[d] == 1 => {}
                    _ => escaped.push(off),
                }
            }
        }
        // What an escaped address may reach: its variable (a named object), else the rest of the frame.
        let mut hit: Vec<(u32, u32)> = Vec::new();
        for &e in &escaped {
            let l = f
                .locals
                .iter()
                .find(|l| l.slot <= e && e < l.slot.saturating_add(l.size));
            hit.push(l.map_or((e, f.frame), |l| (l.slot, l.slot.saturating_add(l.size))));
        }
        let mut slots: Vec<(u32, Width)> = Vec::new();
        let mut bad: Vec<(u32, u32)> = Vec::new();
        for &(off, _, w) in &cand {
            let end = off.checked_add(w.bytes())?;
            let clash = |a: u32, b: u32| a < end && off < b;
            if end > f.frame || hit.iter().any(|&(a, b)| clash(a, b)) {
                bad.push((off, end));
                continue;
            }
            match slots.iter().find(|s| s.0 == off) {
                Some(s) if s.1 == w => {}
                Some(_) => bad.push((off, end)),
                None => slots.push((off, w)),
            }
        }
        // Overlapping variables of different offsets or widths: neither.
        let overlap =
            |a: &(u32, Width), b: &(u32, Width)| a.0 < b.0 + b.1.bytes() && b.0 < a.0 + a.1.bytes();
        let all: Vec<(u32, Width)> = slots.clone();
        slots.retain(|s| {
            !bad.iter().any(|&(a, b)| s.0 < b && a < s.0 + s.1.bytes())
                && !all.iter().any(|o| o != s && overlap(o, s))
        });
        // The accesses; only slots accessed in a loop, the first MAX_SLOTS of them.
        let mut at = vec![None; n];
        let mut hot: Vec<u32> = vec![0; slots.len()];
        let mut stored = vec![false; slots.len()];
        let by_reg: std::collections::HashMap<u32, usize> = cand
            .iter()
            .filter_map(|&(off, r, _)| slots.iter().position(|s| s.0 == off).map(|k| (r.0, k)))
            .collect();
        for (pc, i) in f.code.iter().enumerate() {
            if let Inst::Load { addr, .. } | Inst::Store { addr, .. } = *i
                && let Some(&k) = by_reg.get(&addr.0)
            {
                at[pc] = Some(k as u32);
                hot[k] += u32::from(in_loop[pc]);
                stored[k] |= matches!(i, Inst::Store { .. });
            }
        }
        let mut keep: Vec<usize> = (0..slots.len()).filter(|&k| hot[k] > 0).collect();
        keep.sort_by_key(|&k| std::cmp::Reverse(hot[k]));
        keep.truncate(MAX_SLOTS);
        if keep.is_empty() {
            return None;
        }
        let renum = |k: u32| keep.iter().position(|&x| x as u32 == k).map(|x| x as u32);
        for a in &mut at {
            *a = a.and_then(renum);
        }
        Some(Scalars {
            at,
            slots: keep.iter().map(|&k| slots[k]).collect(),
            stored: keep.iter().map(|&k| stored[k]).collect(),
        })
    }
}

/// The registers some instruction of `f` defines as a deferred value (`Gen::std`): the others
/// are always in the frame, so no phi carries them.
pub fn deferrable(f: &Func) -> Vec<bool> {
    let mut out = vec![false; f.reg_ty.len()];
    for i in &f.code {
        if matches!(
            i,
            Inst::Const { .. }
                | Inst::ConstF { .. }
                | Inst::Move { .. }
                | Inst::Select { .. }
                | Inst::Bin { .. }
                | Inst::BinRange { .. }
                | Inst::Un { .. }
                | Inst::FBin { .. }
                | Inst::FUn { .. }
                | Inst::IToF { .. }
                | Inst::Cmp { .. }
                | Inst::FCmp { .. }
                | Inst::Bool { .. }
                | Inst::AddrOf { .. }
                | Inst::Load { .. }
                | Inst::SigVal { .. }
        ) {
            i.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(x) = out.get_mut(r.0 as usize)
                {
                    *x = true;
                    // A signal reference is two registers.
                    if matches!(i, Inst::SigVal { .. })
                        && let Some(y) = out.get_mut(r.0 as usize + 1)
                    {
                        *y = true;
                    }
                }
            });
        }
    }
    out
}

/// The address space an address register is known to point into at a block start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sp {
    Inst,
    Temp,
    Heap,
    /// Probably a heap object: a pointer loaded from the slot of a variable (#768).
    Likely,
}

/// Registers that hold an address of instance data, the temporary area or a heap object (or
/// probably one, the pointer loaded from a variable's slot) at the
/// start of each block, whatever path reached it (#684): a forward dataflow over the blocks
/// (`starts[pc]`), meet = both sides agree, entry = the registers the interpreter preloads.
/// The result is indexed by the block's first pc. `None` for a function too big for it.
pub fn spaces(prog: &Program, f: &Func, starts: &[bool]) -> Option<Vec<Vec<(u32, Sp)>>> {
    let n = f.code.len();
    // ponytail: bits per block = candidates x 5; a function over this size keeps block-local facts.
    const MAX_BITS: usize = 1 << 27;
    const SETS: usize = 5;
    let nregs = f.reg_ty.len();
    let mut cand = vec![u32::MAX; nregs];
    let mut regs: Vec<u32> = Vec::new();
    let mut defs = vec![0u8; nregs];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
        if let Inst::AddrOf { dst, .. }
        | Inst::Alloc { dst, .. }
        | Inst::NewFrame { dst, .. }
        | Inst::Deref { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::Index { dst, .. }
        | Inst::Load { dst, .. } = *i
            && cand.get(dst.0 as usize) == Some(&u32::MAX)
        {
            cand[dst.0 as usize] = regs.len() as u32;
            regs.push(dst.0);
        }
    }
    let nb = starts.iter().filter(|&&s| s).count();
    let w = regs.len().div_ceil(64);
    if regs.is_empty() || nb.checked_mul(SETS * w * 64)? > MAX_BITS {
        return None;
    }
    let sets = SETS * w;
    // Bit `k` of set `s` (0 instance data, 1 temporary area, 2 heap, 3 the address of a
    // variable's slot, 4 probably heap).
    let bit = |st: &[u64], s: usize, k: usize| st[s * w + k / 64] >> (k % 64) & 1 != 0;
    let put = |st: &mut [u64], s: usize, k: usize, on: bool| {
        let m = 1u64 << (k % 64);
        if on {
            st[s * w + k / 64] |= m;
        } else {
            st[s * w + k / 64] &= !m;
        }
    };
    let mut seed = vec![0u64; sets];
    for i in &f.code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Inst,
            ..
        } = *i
            && defs.get(dst.0 as usize) == Some(&1)
        {
            put(&mut seed, 0, cand[dst.0 as usize] as usize, true);
        }
    }
    let mut ins: Vec<Option<Vec<u64>>> = vec![None; n + 1];
    ins[0] = Some(seed);
    let mut work = vec![0usize];
    let mut guard = 0usize;
    while let Some(l) = work.pop() {
        guard += 1;
        if guard > 200 * nb + 1000 {
            return None;
        }
        let mut st = ins[l].clone()?;
        let mut pc = l;
        loop {
            let i = &f.code[pc];
            let from = |st: &[u64], r: Reg| {
                cand.get(r.0 as usize)
                    .filter(|&&k| k != u32::MAX)
                    .map(|&k| {
                        let k = k as usize;
                        std::array::from_fn(|s| bit(st, s, k))
                    })
                    .unwrap_or([false; SETS])
            };
            let new = match *i {
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Inst,
                    ..
                } => Some((dst, [true, false, false, false, false])),
                Inst::Alloc { dst, .. } | Inst::NewFrame { dst, .. } => {
                    Some((dst, [false, true, false, false, false]))
                }
                Inst::Deref { dst, .. } => Some((dst, [false, false, true, false, false])),
                // A variable's slot (not a parameter's: the caller's argument is in any space).
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Frame,
                    off,
                } if !f.params.iter().any(|p| u64::from(p.slot) == u64::from(off)) => {
                    Some((dst, [false, false, false, true, false]))
                }
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Up(1..=2),
                    ..
                } => Some((dst, [false, false, false, true, false])),
                Inst::Move { dst, src } => Some((dst, from(&st, src))),
                Inst::Index { dst, base, .. } => Some((dst, from(&st, base))),
                Inst::Load {
                    dst,
                    addr,
                    w: Width::B8,
                } => Some((dst, [false, false, false, false, from(&st, addr)[3]])),
                _ => None,
            };
            i.visit(&mut |r, role, _| {
                if role == Role::Def {
                    for k in [r.0, r.0.saturating_add(1)] {
                        if let Some(&c) = cand.get(k as usize)
                            && c != u32::MAX
                        {
                            for s in 0..SETS {
                                put(&mut st, s, c as usize, false);
                            }
                        }
                    }
                }
            });
            if let Some((d, v)) = new {
                for (s, &on) in v.iter().enumerate() {
                    put(&mut st, s, cand[d.0 as usize] as usize, on);
                }
            }
            let next = succs(prog, f, pc, i)?;
            if let [s] = next[..]
                && s as usize == pc + 1
                && !starts[pc + 1]
            {
                pc += 1;
                continue;
            }
            for s in next {
                let s = s as usize;
                if !starts[s] {
                    return None;
                }
                let changed = match &mut ins[s] {
                    None => {
                        ins[s] = Some(st.clone());
                        true
                    }
                    Some(old) => {
                        let mut ch = false;
                        for (o, &x) in old.iter_mut().zip(&st) {
                            if *o & x != *o {
                                *o &= x;
                                ch = true;
                            }
                        }
                        ch
                    }
                };
                if changed {
                    work.push(s);
                }
            }
            break;
        }
    }
    Some(
        (0..=n)
            .map(|l| {
                let Some(st) = &ins[l] else {
                    return Vec::new();
                };
                let mut v = Vec::new();
                for (k, &r) in regs.iter().enumerate() {
                    for (s, sp) in [(0, Sp::Inst), (1, Sp::Temp), (2, Sp::Heap), (4, Sp::Likely)] {
                        if bit(st, s, k) {
                            v.push((r, sp));
                        }
                    }
                }
                v
            })
            .collect(),
    )
}

/// A process whose activations reach no object of the machine's or the heaps' tags in
/// `Mem::objs` (#766): no call, no access value, no heap object, no package or shared data, and
/// every address register made only from its own frame, the instance data, a constant or the
/// temporary area (a flow-insensitive fixpoint over their definitions). Such an activation skips
/// the table's tags (`rt::enter`, `lean`).
pub fn no_table(f: &Func) -> bool {
    if f.kind != yavs_ir::FuncKind::Process
        || !f.params.is_empty()
        || f.reg_ty.contains(&RegTy::Handle)
    {
        return false;
    }
    let n = f.reg_ty.len();
    let is_ptr = |r: Reg| f.reg_ty.get(r.0 as usize) == Some(&RegTy::Ptr);
    // Per address register: still of a known space, and defined somewhere.
    let mut ok = vec![true; n];
    let mut defined = vec![false; n];
    let good = |ok: &[bool], r: Reg| !is_ptr(r) || ok[r.0 as usize];
    loop {
        let mut changed = false;
        for i in &f.code {
            let safe = match *i {
                Inst::Call { .. }
                | Inst::CallForeign { .. }
                | Inst::CallIndirect { .. }
                | Inst::CallProt { .. }
                | Inst::New { .. }
                | Inst::NewFill { .. }
                | Inst::Free { .. }
                | Inst::Deref { .. }
                | Inst::PkgAlloc { .. }
                | Inst::Elab { .. } => return false,
                Inst::AddrOf { space, .. } => matches!(
                    space,
                    AddrSpace::Frame | AddrSpace::Inst | AddrSpace::Const(_)
                ),
                Inst::Alloc { .. } | Inst::NewFrame { .. } | Inst::LayoutOf { .. } => true,
                Inst::Move { src, .. } => good(&ok, src),
                Inst::Index { base, .. } | Inst::Field { base, .. } => good(&ok, base),
                Inst::Select { a, b, .. } => good(&ok, a) && good(&ok, b),
                // Anything else that makes an address (a load, a native's result) may point
                // anywhere.
                _ => false,
            };
            i.visit(&mut |r, role, _| {
                if role == Role::Def && is_ptr(r) {
                    defined[r.0 as usize] = true;
                    if !safe && ok[r.0 as usize] {
                        ok[r.0 as usize] = false;
                        changed = true;
                    }
                }
            });
        }
        if !changed {
            break;
        }
    }
    // An address register read before any definition is one the interpreter preloaded: unknown.
    let known = |r: Reg| !is_ptr(r) || ok[r.0 as usize] && defined[r.0 as usize];
    f.code.iter().all(|i| {
        let mut all = true;
        i.visit(&mut |r, role, _| all &= role == Role::Def || known(r));
        all && i
            .list()
            .is_none_or(|l| f.regs(l).is_some_and(|rs| rs.iter().all(|&r| known(r))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_diag::{FileId, Span};
    use yavs_ir::{CmpOp, FuncKind, IntOp, Mode, ParamClass, ProgramBuilder, StmtKind};

    /// The liveness of `f` by plain iteration over its instructions: bit sets per instruction.
    fn reference(prog: &Program, f: &Func) -> Vec<Vec<u64>> {
        let (n, words) = (f.code.len(), (f.reg_ty.len() + 1).div_ceil(64).max(1));
        let (mut uses, mut defs) = (vec![vec![0u64; words]; n], vec![vec![0u64; words]; n]);
        for (pc, i) in f.code.iter().enumerate() {
            i.visit(&mut |r, role, _| {
                let row = if role == Role::Def {
                    &mut defs[pc]
                } else {
                    &mut uses[pc]
                };
                row[r.0 as usize / 64] |= 1 << (r.0 % 64);
                if f.reg_ty.get(r.0 as usize) == Some(&RegTy::Sig) {
                    row[(r.0 as usize + 1) / 64] |= 1 << ((r.0 + 1) % 64);
                }
            });
        }
        let mut bits = vec![vec![0u64; words]; n + 1];
        loop {
            let mut changed = false;
            for pc in (0..n).rev() {
                let mut out = vec![0u64; words];
                for s in succs(prog, f, pc, &f.code[pc]).unwrap() {
                    for (o, b) in out.iter_mut().zip(&bits[s as usize]) {
                        *o |= b;
                    }
                }
                for w in 0..words {
                    let v = uses[pc][w] | (out[w] & !defs[pc][w]);
                    changed |= std::mem::replace(&mut bits[pc][w], v) != v;
                }
            }
            if !changed {
                return bits;
            }
        }
    }

    /// A function of `len` straight-line instructions that each define a register, then a loop
    /// with an early exit and a nested branch.
    fn big(len: usize) -> (Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", FuncKind::Function, None);
        let mut fb = pb.define(f);
        let sx = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        let (head, body, odd, step, done) =
            (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let px = fb.addr(AddrSpace::Frame, sx);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: x,
            addr: px,
            w: Width::B8,
        });
        let mut acc = x;
        for k in 0..len {
            let r = fb.reg(RegTy::I64);
            let op = [IntOp::Add, IntOp::Mul, IntOp::Sub][k % 3];
            fb.emit(Inst::Bin {
                op,
                dst: r,
                a: acc,
                b: if k % 7 == 0 { x } else { acc },
            });
            acc = r;
        }
        let (i, one, three) = (fb.int(0), fb.int(1), fb.int(3));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: i,
            b: x,
        });
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        let b = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Rem,
            dst: b,
            a: i,
            b: one,
        });
        fb.emit(Inst::CondBr {
            c: b,
            t: odd,
            f: step,
        });
        fb.at(odd);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: acc,
            a: acc,
            b: three,
        });
        fb.emit(Inst::Br { to: step });
        fb.at(step);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(done);
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
        (pb.finish().unwrap(), f)
    }

    #[test]
    fn the_core_is_what_the_first_block_defines_once_and_keeps_alive() {
        let (prog, id) = big(20);
        let f = prog.funcs.get(id).unwrap();
        let live = Live::new(&prog, f, None).unwrap();
        let (core, from) = entry_core(&prog, f, &live, &preloaded(f));
        // The load of the parameter is read to the end of the loop; the chain's temporaries die
        // at once, the accumulator and the counter are defined again in the loop, and the
        // constants and addresses are preloaded.
        let at = f
            .code
            .iter()
            .position(|i| matches!(i, Inst::Load { .. }))
            .unwrap();
        let Inst::Load { dst, .. } = f.code[at] else {
            unreachable!()
        };
        assert_eq!(core, vec![dst.0]);
        assert_eq!(from as usize, at + 1);
    }

    #[test]
    fn the_core_is_empty_when_a_jump_leads_back_to_the_first_block() {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", FuncKind::Function, None);
        let mut fb = pb.define(f);
        let sx = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        let entry = fb.current();
        let (head, done) = (fb.block(), fb.block());
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let px = fb.addr(AddrSpace::Frame, sx);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: x,
            addr: px,
            w: Width::B8,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        for _ in 0..10 {
            let t = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Add,
                dst: t,
                a: x,
                b: x,
            });
        }
        fb.emit(Inst::CondBr {
            c: x,
            t: done,
            f: entry,
        });
        fb.at(done);
        fb.emit(Inst::Ret { v: Some(x) });
        fb.finish();
        let prog = pb.finish().unwrap();
        let f = prog.funcs.get(f).unwrap();
        let live = Live::new(&prog, f, None).unwrap();
        let (core, _) = entry_core(&prog, f, &live, &preloaded(f));
        assert!(core.is_empty(), "the first block runs again: {core:?}");
    }

    #[test]
    fn rows_match_plain_iteration_for_a_function_over_the_old_cap() {
        // 12 000 registers of 188 words each: 2.2 million words, over the 1 << 20 of before #710.
        let (prog, id) = big(12_000);
        // (The function as built: `Program::func` hands out the optimised one, which folds this.)
        let f = prog.funcs.get(id).unwrap();
        let live = Live::new(&prog, f, None).expect("a function this size gets a liveness matrix");
        assert!(
            f.code.len() * live.words > 1 << 20,
            "under the old cap: {} instructions of {} words",
            f.code.len(),
            live.words
        );
        let want = reference(&prog, f);
        for (pc, w) in want.iter().enumerate() {
            assert_eq!(&live.row(pc)[..w.len()], &w[..], "row {pc}");
        }
    }
}
