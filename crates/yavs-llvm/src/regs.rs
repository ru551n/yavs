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
pub(crate) struct Live {
    words: usize,
    bits: Vec<u64>,
}

/// Functions whose rows would take more than this many words get no deferral.
const MAX_WORDS: usize = 1 << 20;

fn mark(f: &Func, row: &mut [u64], r: Reg) {
    let mut set = |k: usize| {
        if let Some(w) = row.get_mut(k / 64) {
            *w |= 1 << (k % 64);
        }
    };
    set(r.0 as usize);
    if f.reg_ty.get(r.0 as usize) == Some(&RegTy::Sig) {
        set(r.0 as usize + 1);
    }
}

impl Live {
    pub fn row(&self, pc: usize) -> &[u64] {
        &self.bits[pc * self.words..(pc + 1) * self.words]
    }

    /// Backward liveness over the instructions of `f`; `None` for a function too big for it.
    pub fn new(prog: &Program, f: &Func, sc: Option<&Scalars>) -> Option<Live> {
        let n = f.code.len();
        let nv = sc.map_or(0, |s| s.slots.len());
        // One bit past the slot variables is the frame base (`Gen`), never live.
        let words = (f.reg_ty.len() + nv + 1).div_ceil(64).max(1);
        if n.checked_mul(words)? > MAX_WORDS {
            return None;
        }
        // Uses and definitions of each instruction, and its successors.
        let mut uses = vec![0u64; n * words];
        let mut defs = vec![0u64; n * words];
        let mut succ: Vec<Vec<u32>> = Vec::with_capacity(n);
        for (pc, i) in f.code.iter().enumerate() {
            let (u, d) = (
                &mut uses[pc * words..(pc + 1) * words],
                &mut defs[pc * words..(pc + 1) * words],
            );
            i.visit(&mut |r, role, _| {
                mark(f, if role == Role::Def { &mut *d } else { &mut *u }, r)
            });
            if let Some(l) = i.list() {
                for &r in f.regs(l)? {
                    mark(f, &mut *u, r);
                }
            }
            if let Some(sc) = sc {
                let set = |row: &mut [u64], k: usize| {
                    let bit = f.reg_ty.len() + k;
                    row[bit / 64] |= 1 << (bit % 64);
                };
                match (sc.at[pc], i) {
                    (Some(k), Inst::Load { .. }) => set(&mut *u, k as usize),
                    (Some(k), Inst::Store { .. }) => set(&mut *d, k as usize),
                    (_, i) if sc.observes(i) => {
                        for k in (0..nv).filter(|&k| sc.stored[k]) {
                            set(&mut *u, k);
                        }
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
        let mut bits = vec![0u64; (n + 1) * words];
        let mut out = vec![0u64; words];
        for _ in 0..64 {
            let mut changed = false;
            for pc in (0..n).rev() {
                out.fill(0);
                for &s in &succ[pc] {
                    for (o, b) in out.iter_mut().zip(&bits[s as usize * words..]) {
                        *o |= b;
                    }
                }
                for w in 0..words {
                    let v = uses[pc * words + w] | (out[w] & !defs[pc * words + w]);
                    if bits[pc * words + w] != v {
                        bits[pc * words + w] = v;
                        changed = true;
                    }
                }
            }
            if !changed {
                return Some(Live { words, bits });
            }
        }
        None
    }
}

/// The registers the interpreter starts a frame of `f` with a value in: defined exactly once,
/// by a constant or an address that is not an `Up` one (`Pre::new`).
pub(crate) fn preloaded(f: &Func) -> Vec<bool> {
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

/// The instructions that can run after `i` at `pc` (`None`: a branch to a missing block).
pub(crate) fn succs(prog: &Program, f: &Func, pc: usize, i: &Inst) -> Option<Vec<u32>> {
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
pub(crate) struct Reach {
    pub reached: Vec<bool>,
    pub head: Vec<bool>,
    /// The natural loop of each retreating edge: its instructions.
    pub loops: Vec<Vec<usize>>,
}

/// The instructions reached from instruction 0 and from `extra` entry points, and the targets
/// of retreating edges of a depth-first search from them (loop headers: the edge closes a cycle).
pub(crate) fn reach(prog: &Program, f: &Func, extra: &[usize]) -> Option<Reach> {
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
pub(crate) struct Scalars {
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
pub(crate) fn loud(i: &Inst, native: &dyn Fn(&Inst) -> bool) -> bool {
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
pub(crate) fn deferrable(f: &Func) -> Vec<bool> {
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
pub(crate) enum Sp {
    Inst,
    Temp,
    Heap,
}

/// Registers that hold an address of instance data, the temporary area or a heap object at the
/// start of each block, whatever path reached it (#684): a forward dataflow over the blocks
/// (`starts[pc]`), meet = both sides agree, entry = the registers the interpreter preloads.
/// The result is indexed by the block's first pc. `None` for a function too big for it.
pub(crate) fn spaces(prog: &Program, f: &Func, starts: &[bool]) -> Option<Vec<Vec<(u32, Sp)>>> {
    let n = f.code.len();
    // ponytail: bits per block = candidates x 3; a function over this size keeps block-local facts.
    const MAX_BITS: usize = 1 << 27;
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
        | Inst::Deref { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::Index { dst, .. } = *i
            && cand.get(dst.0 as usize) == Some(&u32::MAX)
        {
            cand[dst.0 as usize] = regs.len() as u32;
            regs.push(dst.0);
        }
    }
    let nb = starts.iter().filter(|&&s| s).count();
    let w = regs.len().div_ceil(64);
    if regs.is_empty() || nb.checked_mul(3 * w * 64)? > MAX_BITS {
        return None;
    }
    let sets = 3 * w;
    // Bit `k` of set `s` (0 instance data, 1 temporary area, 2 heap).
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
                        [bit(st, 0, k), bit(st, 1, k), bit(st, 2, k)]
                    })
                    .unwrap_or([false; 3])
            };
            let new = match *i {
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Inst,
                    ..
                } => Some((dst, [true, false, false])),
                Inst::Alloc { dst, .. } => Some((dst, [false, true, false])),
                Inst::Deref { dst, .. } => Some((dst, [false, false, true])),
                Inst::Move { dst, src } => Some((dst, from(&st, src))),
                Inst::Index { dst, base, .. } => Some((dst, from(&st, base))),
                _ => None,
            };
            i.visit(&mut |r, role, _| {
                if role == Role::Def {
                    for k in [r.0, r.0.saturating_add(1)] {
                        if let Some(&c) = cand.get(k as usize)
                            && c != u32::MAX
                        {
                            for s in 0..3 {
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
                    for (s, sp) in [(0, Sp::Inst), (1, Sp::Temp), (2, Sp::Heap)] {
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
