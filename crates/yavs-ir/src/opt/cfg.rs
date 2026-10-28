//! Clean-up of the finished code of a subprogram that cannot suspend (#444) and of a process
//! (#791: the block it pauses at is never copied or jumped past).
//!
//! The passes before this one work on the walk's tables; the shapes lowering and promotion
//! leave behind are easier to see on the final code: a value computed into a temporary and
//! moved into a variable (`i := i + 1` is an add and a move), and conditions kept in a
//! register only to be tested by the next block. Each pass here is linear and runs once.

// Loops index several parallel per-block vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Counts of definitions and readers per register.
fn counts(f: &crate::Func) -> (Vec<u8>, Vec<u32>) {
    let n = f.reg_ty.len();
    let (mut defs, mut uses) = (vec![0u8; n], vec![0u32; n]);
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            let r = r.0 as usize;
            match role {
                Role::Def => defs[r] = defs[r].saturating_add(1),
                Role::Use => uses[r] += 1,
            }
        });
        if let Some(l) = i.list() {
            for r in f.regs(l).unwrap_or(&[]) {
                uses[r.0 as usize] += 1;
            }
        }
    }
    (defs, uses)
}

/// Remove the instructions marked dead, keeping the blocks contiguous.
fn squeeze(f: &mut crate::Func) {
    let (mut code, mut loc) = (
        Vec::with_capacity(f.code.len()),
        Vec::with_capacity(f.code.len()),
    );
    let mut ranges = Vec::with_capacity(f.blocks.len());
    for b in f.blocks.values() {
        let start = code.len() as u32;
        for pc in b.start as usize..b.end as usize {
            if !is_dead(&f.code[pc]) {
                code.push(f.code[pc]);
                loc.push(f.loc[pc]);
            }
        }
        ranges.push((start, code.len() as u32));
    }
    for (i, (s, e)) in ranges.into_iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
}

/// Instructions whose one result register can be renamed, without losing a fusion the
/// predecoder makes of a compare and its branch.
fn coalescable(i: &Inst) -> bool {
    retargetable(i) && !matches!(i, Inst::Cmp { .. } | Inst::Bool { .. })
}

/// `t = a op b; p = t` with nothing else reading `t` and nothing touching `p` in between:
/// the first instruction writes `p` and the move goes. Most of these are `i := i + 1` whose
/// range check was dropped after promotion had looked for them.
fn coalesce(f: &mut crate::Func, defs: &[u8], uses: &[u32]) -> bool {
    let mut any = false;
    // Per register: the instruction that last defined it, and the last one that read or
    // wrote it, each with the block it is in.
    let n = f.reg_ty.len();
    let mut def_at = vec![(u32::MAX, 0u32); n];
    let mut touch = vec![(u32::MAX, 0u32); n];
    for b in 0..f.blocks.len() as u32 {
        let blk = f.blocks.values()[b as usize];
        for pc in blk.start..blk.end {
            let i = f.code[pc as usize];
            if let Inst::Move { dst: p, src: s } = i
                && p != s
                && defs[s.0 as usize] == 1
                && uses[s.0 as usize] == 1
                && f.reg_ty[s.0 as usize] == f.reg_ty[p.0 as usize]
                && let (db, pd) = def_at[s.0 as usize]
                && db == b
                && coalescable(&f.code[pd as usize])
                && (touch[p.0 as usize].0 != b || touch[p.0 as usize].1 <= pd)
            {
                f.code[pd as usize].visit_mut(&mut |r, role, _| {
                    if role == Role::Def && *r == s {
                        *r = p;
                    }
                });
                f.code[pc as usize] = dead();
                def_at[p.0 as usize] = (b, pd);
                touch[p.0 as usize] = (b, pc);
                any = true;
                continue;
            }
            let mut mark = |r: Reg, def: bool| {
                touch[r.0 as usize] = (b, pc);
                if def {
                    def_at[r.0 as usize] = (b, pc);
                }
            };
            i.visit(&mut |r, role, _| mark(r, role == Role::Def));
            if let Some(l) = i.list() {
                for &r in f.regs(l).unwrap_or(&[]) {
                    mark(r, false);
                }
            }
        }
    }
    if any {
        squeeze(f);
    }
    any
}

/// Most instructions (besides the terminator) of a block that is copied into its predecessors.
const MAX_COPY: usize = 3;
/// Blocks one block takes over in a row.
const MAX_HOPS: usize = 6;

type Blk = (Vec<Inst>, Vec<crate::StmtId>);

/// Cheap instructions without effects: what a copied block may hold besides its branch.
fn dupable(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Move { .. }
            | Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Cmp { .. }
            | Inst::Bool { .. }
            | Inst::Select { .. }
    )
}

fn small(c: &[Inst]) -> bool {
    let n = c.len();
    (1..=MAX_COPY + 1).contains(&n)
        && c[..n - 1].iter().all(dupable)
        && matches!(
            c[n - 1],
            Inst::Br { .. } | Inst::CondBr { .. } | Inst::Ret { .. }
        )
}

/// A loop test followed by its step block (`[Bin add/sub, Br]`): the predecoder makes one
/// operation of the three, which needs them in this order. In a process (`wide`) the step block
/// can still hold a checked add or a move into the variable: any short block that jumps back
/// with an add or subtract counts.
fn latch(code: &[Inst], next: Option<&[Inst]>, l: usize, wide: bool) -> bool {
    let step = |i: &Inst| {
        matches!(
            i,
            Inst::Bin {
                op: IntOp::Add | IntOp::Sub,
                ..
            }
        ) || wide
            && matches!(
                i,
                Inst::BinRange {
                    op: IntOp::Add | IntOp::Sub,
                    ..
                }
            )
    };
    matches!(code.last(), Some(Inst::CondBr { t, f, .. }) if t.0 as usize == l + 1 || f.0 as usize == l + 1)
        && next.is_some_and(|s| match s {
            [i, Inst::Br { .. }] => step(i),
            _ => {
                wide && s.len() <= 4
                    && matches!(s.last(), Some(Inst::Br { to }) if to.0 as usize <= l)
                    && s.iter().any(step)
            }
        })
}

/// Does a block read or define a register defined once by a constant or an address?
fn reads_konst(code: &[Inst], konst: &[bool]) -> bool {
    code.iter().any(|i| {
        let mut hit = false;
        i.visit(&mut |r, _, _| hit |= konst.get(r.0 as usize).copied().unwrap_or(false));
        hit
    })
}

/// The blocks of a function as `thread` changes them: the original code, except for the
/// blocks it rewrote.
struct Blocks<'a> {
    f: &'a crate::Func,
    changed: FxMap<usize, Blk>,
}

impl Blocks<'_> {
    fn code(&self, b: usize) -> &[Inst] {
        match self.changed.get(&b) {
            Some(c) => &c.0,
            None => {
                let r = self.f.blocks.values()[b];
                &self.f.code[r.start as usize..r.end as usize]
            }
        }
    }

    /// The block's code and statements, for writing.
    fn edit(&mut self, b: usize) -> &mut Blk {
        let f = self.f;
        self.changed.entry(b).or_insert_with(|| {
            let r = f.blocks.values()[b];
            let rg = r.start as usize..r.end as usize;
            (f.code[rg.clone()].to_vec(), f.loc[rg].to_vec())
        })
    }
}

/// Let a block that ends in a jump to a small block take over that block's code, so that a
/// short-circuit chain or a loop test is decided where its inputs are known: `t := a; if t
/// then t := b; end if; if t then ...` becomes a branch on `a` and a branch on `b`.
/// Returns whether anything changed.
fn thread(prog: &Program, f: &mut crate::Func, defs: &[u8], uses: &[u32]) -> bool {
    let n0 = f.reg_ty.len();
    // Registers defined once by a constant.
    let mut kc: Vec<Option<i64>> = vec![None; n0];
    for i in &f.code {
        if let Inst::Const { dst, v } = *i
            && defs[dst.0 as usize] == 1
        {
            kc[dst.0 as usize] = Some(v);
        }
    }
    let nb = f.blocks.len();
    // The block a process pauses at before its first run (`Func::body`) stays where it is.
    let pause = f.body.map(|b| b.0 as usize);
    // A process: the registers defined once by a constant or an address. The frame starts with
    // their values, but no register of a process may be read across a suspension, so a block that
    // reads one (or defines one) is not copied: its copy would need definitions of its own, which
    // cost an operation where the frame's value cost none.
    let mut kdef: Vec<bool> = Vec::new();
    if pause.is_some() {
        kdef = vec![false; n0];
        for i in &f.code {
            if let Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. } =
                *i
                && defs[dst.0 as usize] == 1
                && !matches!(
                    i,
                    Inst::AddrOf {
                        space: AddrSpace::Up(_),
                        ..
                    }
                )
            {
                kdef[dst.0 as usize] = true;
            }
        }
    }
    let mut bl = Blocks {
        f,
        changed: FxMap::default(),
    };
    let mut new_regs: Vec<RegTy> = Vec::new();
    let budget = f.code.len() / 2 + 32;
    let mut grown = 0usize;
    for a in 0..nb {
        for _ in 0..MAX_HOPS {
            let Some(&last) = bl.code(a).last() else {
                break;
            };
            match last {
                Inst::CondBr { c, t, f: e } => {
                    // What is known of registers at the end of `a`: constants and copies.
                    let body = bl.code(a).len() - 1;
                    let mut known: Vec<(u32, i64)> = Vec::new();
                    let mut copy: Vec<(u32, Reg)> = Vec::new();
                    for i in &bl.code(a)[..body] {
                        note(&mut known, &mut copy, i, &kc, &f.reg_ty);
                    }
                    let c = copy.iter().find(|x| x.0 == c.0).map_or(c, |x| x.1);
                    let v = known
                        .iter()
                        .find(|x| x.0 == c.0)
                        .map(|x| x.1)
                        .or(kc.get(c.0 as usize).copied().flatten());
                    let now = if let Some(v) = v {
                        Inst::Br {
                            to: if v != 0 { t } else { e },
                        }
                    } else {
                        // An edge into a block that only tests the same register again.
                        let (mut nt, mut ne) = (t, e);
                        for (edge, truth) in [(&mut nt, true), (&mut ne, false)] {
                            if let [
                                Inst::CondBr {
                                    c: c2,
                                    t: t2,
                                    f: f2,
                                },
                            ] = *bl.code(edge.0 as usize)
                                && c2 == c
                                && edge.0 as usize != a
                                && Some(edge.0 as usize) != pause
                            {
                                *edge = if truth { t2 } else { f2 };
                            }
                        }
                        Inst::CondBr { c, t: nt, f: ne }
                    };
                    if now == last {
                        break;
                    }
                    *bl.edit(a).0.last_mut().unwrap() = now;
                    // A jump goes on with the block it jumps to; a branch is done.
                    if matches!(now, Inst::CondBr { .. }) {
                        break;
                    }
                }
                Inst::Br { to } => {
                    let l = to.0 as usize;
                    if l == a
                        || Some(l) == pause
                        || pause.is_some() && reads_konst(bl.code(l), &kdef)
                        || l >= nb
                        || grown > budget
                        || !small(bl.code(l))
                        || latch(
                            bl.code(l),
                            (l + 1 < nb).then(|| bl.code(l + 1)),
                            l,
                            pause.is_some(),
                        )
                    {
                        break;
                    }
                    let lc = bl.code(l).to_vec();
                    let ll = match bl.changed.get(&l) {
                        Some(c) => c.1.clone(),
                        None => {
                            let r = f.blocks.values()[l];
                            f.loc[r.start as usize..r.end as usize].to_vec()
                        }
                    };
                    // Registers that only this block uses and defines once get fresh
                    // names in the copy, so that the compare and its branch stay fusable.
                    let mut local: Vec<(u32, u32)> = Vec::new();
                    let mut cnt: Vec<(u32, u32)> = Vec::new();
                    for i in &lc {
                        i.visit(&mut |r, role, _| {
                            if role == Role::Use {
                                match cnt.iter_mut().find(|x| x.0 == r.0) {
                                    Some(x) => x.1 += 1,
                                    None => cnt.push((r.0, 1)),
                                }
                            }
                        });
                    }
                    let blk = bl.edit(a);
                    blk.0.pop();
                    blk.1.pop();
                    for (k, i) in lc.iter().enumerate() {
                        let mut i = *i;
                        i.visit_mut(&mut |r, role, _| {
                            if role == Role::Use
                                && let Some(m) = local.iter().find(|x| x.0 == r.0)
                            {
                                r.0 = m.1;
                            }
                        });
                        let mut skip = false;
                        let mut fresh_for: Option<(Reg, RegTy)> = None;
                        i.visit(&mut |r, role, _| {
                            if role == Role::Def && (r.0 as usize) < n0 {
                                let d = r.0 as usize;
                                if defs[d] == 1
                                    && matches!(i, Inst::Const { .. } | Inst::ConstF { .. })
                                {
                                    skip = true;
                                } else if defs[d] == 1
                                    && uses[d] == cnt.iter().find(|x| x.0 == r.0).map_or(0, |x| x.1)
                                    && k + 1 < lc.len()
                                {
                                    fresh_for = Some((r, f.reg_ty[d]));
                                }
                            }
                        });
                        if skip {
                            continue;
                        }
                        if let Some((old, ty)) = fresh_for {
                            let nr = Reg((n0 + new_regs.len()) as u32);
                            new_regs.push(ty);
                            local.push((old.0, nr.0));
                            i.visit_mut(&mut |r, role, _| {
                                if role == Role::Def && *r == old {
                                    *r = nr;
                                }
                            });
                        }
                        blk.0.push(i);
                        blk.1.push(ll[k]);
                        grown += 1;
                    }
                }
                _ => break,
            }
        }
    }
    if bl.changed.is_empty() {
        return false;
    }
    let changed = std::mem::take(&mut bl.changed);
    emit(prog, f, changed, new_regs, defs);
    true
}

/// Write the rewritten blocks back. The blocks no branch reaches any more are reduced to a
/// jump to the next block, which the predecoder runs through (as `Opt::kill_unreachable`
/// does); a constant or address defined once in one of them is a value the frame starts
/// with and other blocks rely on, so its definition moves to the entry (in a process no
/// reachable block reads such a definition of an unreachable block: it would be read before
/// any definition in the activation).
fn emit(
    prog: &Program,
    f: &mut crate::Func,
    changed: FxMap<usize, Blk>,
    new_regs: Vec<RegTy>,
    defs: &[u8],
) {
    let nb = f.blocks.len();
    let get = |b: usize| -> (&[Inst], &[crate::StmtId]) {
        match changed.get(&b) {
            Some(c) => (&c.0, &c.1),
            None => {
                let r = f.blocks.values()[b];
                let rg = r.start as usize..r.end as usize;
                (&f.code[rg.clone()], &f.loc[rg])
            }
        }
    };
    let mut seen = vec![false; nb];
    let mut stack = vec![0usize];
    while let Some(b) = stack.pop() {
        if std::mem::replace(&mut seen[b], true) {
            continue;
        }
        match get(b).0.last() {
            Some(Inst::Case { table, .. }) => {
                if let Some(t) = prog.cases.get(*table) {
                    stack.extend(
                        t.arms
                            .iter()
                            .map(|a| a.2.0 as usize)
                            .chain(std::iter::once(t.default.0 as usize))
                            .filter(|&t| t < nb),
                    );
                }
            }
            Some(i) => stack.extend(i.targets().iter().map(|t| t.0 as usize).filter(|&t| t < nb)),
            None => {}
        }
    }
    let mut code: Vec<Inst> = Vec::with_capacity(f.code.len());
    let mut loc: Vec<crate::StmtId> = Vec::with_capacity(f.code.len());
    let mut ranges = Vec::with_capacity(nb);
    let mut keep: Vec<(Inst, crate::StmtId)> = Vec::new();
    for b in 0..nb {
        let (c, l) = get(b);
        let start = code.len() as u32;
        let jump = matches!(c.last(), Some(Inst::Br { .. } | Inst::CondBr { .. }));
        if seen[b] || b + 1 >= nb || (c.len() <= 1 && !jump) {
            code.extend_from_slice(c);
            loc.extend_from_slice(l);
        } else {
            for (i, st) in c.iter().zip(l) {
                if let Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. } =
                    *i
                    && defs.get(dst.0 as usize) == Some(&1)
                {
                    keep.push((*i, *st));
                }
            }
            code.push(Inst::Br {
                to: BlockId(b as u32 + 1),
            });
            loc.push(l[0]);
        }
        ranges.push((start, code.len() as u32));
    }
    if !keep.is_empty() {
        let n = keep.len() as u32;
        let (ci, cl): (Vec<Inst>, Vec<crate::StmtId>) = keep.into_iter().unzip();
        code.splice(0..0, ci);
        loc.splice(0..0, cl);
        for r in &mut ranges {
            r.0 += n;
            r.1 += n;
        }
        ranges[0].0 = 0;
    }
    for (i, (s, e)) in ranges.into_iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
    f.reg_ty.extend(new_regs);
}

/// Track constants and copies through straight-line code.
fn note(
    known: &mut Vec<(u32, i64)>,
    copy: &mut Vec<(u32, Reg)>,
    i: &Inst,
    kc: &[Option<i64>],
    tys: &[RegTy],
) {
    let mut defd: [u32; 2] = [u32::MAX; 2];
    let mut n = 0;
    i.visit(&mut |r, role, _| {
        if role == Role::Def {
            if n < 2 {
                defd[n] = r.0;
            }
            n += 1;
        }
    });
    if n > 2 {
        known.clear();
        copy.clear();
    }
    for &d in defd.iter().take(n.min(2)) {
        known.retain(|x| x.0 != d);
        copy.retain(|x| x.0 != d && x.1.0 != d);
    }
    match *i {
        Inst::Const { dst, v } => known.push((dst.0, v)),
        Inst::Move { dst, src } if dst != src => {
            let src = copy.iter().find(|x| x.0 == src.0).map_or(src, |x| x.1);
            if let Some(v) = known
                .iter()
                .find(|x| x.0 == src.0)
                .map(|x| x.1)
                .or(kc.get(src.0 as usize).copied().flatten())
            {
                known.push((dst.0, v));
            }
            if tys.get(dst.0 as usize) == tys.get(src.0 as usize)
                && matches!(tys.get(dst.0 as usize), Some(RegTy::I64 | RegTy::Ptr))
            {
                copy.push((dst.0, src));
            }
        }
        // A condition of known operands (`Not` of a flag set a few instructions before).
        Inst::Bool { op, dst, a, b } => {
            let val = |r: Reg| {
                known
                    .iter()
                    .find(|x| x.0 == r.0)
                    .map(|x| x.1)
                    .or(kc.get(r.0 as usize).copied().flatten())
            };
            let v = match (op, val(a), val(b)) {
                (BoolOp::Not, Some(x), _) => Some(x == 0),
                (BoolOp::And, Some(x), Some(y)) => Some(x != 0 && y != 0),
                (BoolOp::Or, Some(x), Some(y)) => Some(x != 0 || y != 0),
                (BoolOp::Xor, Some(x), Some(y)) => Some((x != 0) != (y != 0)),
                _ => None,
            };
            if let Some(v) = v {
                known.push((dst.0, i64::from(v)));
            }
        }
        _ => {}
    }
}

/// The register an instruction defines when it can go if nothing reads it: what
/// `removable_dst` allows, a length, and the allocation of a descriptor block and the
/// descriptor written into it.
fn droppable(i: &Inst) -> Option<Reg> {
    match *i {
        Inst::DopeLen { dst, .. } | Inst::Alloc { dst, .. } => Some(dst),
        // (Only a register defined on several paths is dropped from: the frame starts with
        // the value of one defined once.)
        Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. } => Some(dst),
        _ => removable_dst(i),
    }
}

/// Pure definitions no path reads (the moves and flags `thread` leaves behind, the length
/// and descriptor of a slice whose test `slice_tests` took apart).
fn dead_defs(prog: &Program, f: &mut crate::Func) -> bool {
    let n = f.reg_ty.len();
    let nb = f.blocks.len();
    let mut idx = vec![u32::MAX; n];
    let mut m = 0usize;
    let mut any = false;
    // Allocations and descriptor writes whose block is read, between the allocation and
    // the next one of the register, by nothing but descriptor writes and length reads: no
    // copy of its address can exist (a later reader makes the register live, which keeps it).
    let mut okdef = vec![false; f.code.len()];
    let has_dope = f.code.iter().any(|i| matches!(i, Inst::MakeDope { .. }));
    for blk in f.blocks.values().iter().filter(|_| has_dope) {
        let (s0, e0) = (blk.start as usize, blk.end as usize);
        for pc in s0..e0 {
            if let Inst::Alloc { dst, .. } = f.code[pc] {
                let mut ok = true;
                for q in pc + 1..e0 {
                    let i = f.code[q];
                    for_uses(f, &i, |r| {
                        if r == dst
                            && !matches!(i, Inst::MakeDope { at, .. } if at == dst)
                            && !matches!(i, Inst::DopeLen { dope, .. } if dope == dst)
                        {
                            ok = false;
                        }
                    });
                    let mut redef = false;
                    for_defs(&i, |r| redef |= r == dst);
                    if !ok || redef {
                        break;
                    }
                }
                okdef[pc] = ok;
                // The descriptor writes that follow, up to the next allocation.
                for q in pc + 1..e0 {
                    let mut redef = false;
                    for_defs(&f.code[q], |r| redef |= r == dst);
                    if redef {
                        break;
                    }
                    if let Inst::MakeDope { at, .. } = f.code[q]
                        && at == dst
                    {
                        okdef[q] = ok;
                    }
                }
            }
        }
    }
    // Registers defined on several paths (the flags `thread` leaves behind), and the
    // lengths and blocks of descriptors.
    let (defs, uses) = counts(f);
    for i in &f.code {
        // (What nothing reads, such as the flag a decided branch leaves, goes by itself.)
        if droppable(i).is_some_and(|d| uses[d.0 as usize] == 0) {
            any = true;
        }
        let d = match i {
            Inst::MakeDope { at, .. } => Some(*at),
            _ => droppable(i).filter(|d| {
                defs[d.0 as usize] > 1 || matches!(i, Inst::DopeLen { .. } | Inst::Alloc { .. })
            }),
        };
        if let Some(d) = d
            && matches!(f.reg_ty[d.0 as usize], RegTy::I64 | RegTy::Ptr)
        {
            any = true;
            if idx[d.0 as usize] == u32::MAX {
                idx[d.0 as usize] = m as u32;
                m += 1;
            }
        }
    }
    // (The sets are as wide as the registers tracked, not all of them: a large process has
    // thousands of blocks and registers, #791.)
    if !any || nb * m > 1 << 24 {
        return false;
    }
    let succ = super::memo::successors(prog, f);
    let w = m.div_ceil(64).max(1);
    let mut gen_: Vec<Vec<u64>> = vec![vec![0; w]; nb];
    let mut kill: Vec<Vec<u64>> = vec![vec![0; w]; nb];
    let set = |v: &mut Vec<u64>, k: u32| v[k as usize / 64] |= 1 << (k % 64);
    for b in 0..nb {
        let blk = f.blocks.values()[b];
        for pc in blk.start as usize..blk.end as usize {
            let i = f.code[pc];
            for_uses(f, &i, |r| {
                let k = idx[r.0 as usize];
                if k != u32::MAX && kill[b][k as usize / 64] >> (k % 64) & 1 == 0 {
                    set(&mut gen_[b], k);
                }
            });
            for_defs(&i, |r| {
                let k = idx[r.0 as usize];
                if k != u32::MAX {
                    set(&mut kill[b], k);
                }
            });
        }
    }
    let mut live_in = gen_.clone();
    let mut live_out: Vec<Vec<u64>> = vec![vec![0; w]; nb];
    let mut again = true;
    while again {
        again = false;
        for b in (0..nb).rev() {
            let mut out = vec![0u64; w];
            for &s in &succ[b] {
                if s < nb {
                    for (o, i) in out.iter_mut().zip(&live_in[s]) {
                        *o |= *i;
                    }
                }
            }
            let inn: Vec<u64> = (0..w)
                .map(|x| gen_[b][x] | (out[x] & !kill[b][x]))
                .collect();
            if inn != live_in[b] || out != live_out[b] {
                live_in[b] = inn;
                live_out[b] = out;
                again = true;
            }
        }
    }
    let mut removed = false;
    for b in 0..nb {
        let blk = f.blocks.values()[b];
        let mut live = live_out[b].clone();
        for pc in (blk.start as usize..blk.end as usize).rev() {
            let i = f.code[pc];
            // A descriptor written into a block nothing reads afterwards.
            let target = match i {
                Inst::MakeDope { at, .. } => Some(at),
                _ => droppable(&i),
            };
            if let Some(d) = target {
                if uses[d.0 as usize] == 0
                    && !matches!(i, Inst::MakeDope { .. } | Inst::Alloc { .. })
                {
                    f.code[pc] = dead();
                    removed = true;
                    continue;
                }
                let k = idx[d.0 as usize];
                // (`MakeDope` reads its block's address: dead means no later reader.)
                let block = matches!(i, Inst::MakeDope { .. } | Inst::Alloc { .. });
                if k != u32::MAX
                    && (!block || okdef[pc])
                    && live[k as usize / 64] >> (k % 64) & 1 == 0
                {
                    f.code[pc] = dead();
                    removed = true;
                    continue;
                }
            }
            for_defs(&i, |r| {
                let k = idx[r.0 as usize];
                if k != u32::MAX {
                    live[k as usize / 64] &= !(1 << (k % 64));
                }
            });
            for_uses(f, &i, |r| {
                let k = idx[r.0 as usize];
                if k != u32::MAX {
                    live[k as usize / 64] |= 1 << (k % 64);
                }
            });
        }
    }
    if removed {
        squeeze(f);
    }
    removed
}

/// `n := s'length` of a slice is tested against zero right after the descriptor of the
/// slice is built (`if s(a to b)'length = 0`): the test is a comparison of the bounds, and
/// the descriptor, its block and the length go when nothing else reads them.
fn slice_tests(f: &mut crate::Func, defs: &[u8]) -> bool {
    let n = f.reg_ty.len();
    let mut kc: Vec<Option<i64>> = vec![None; n];
    for i in &f.code {
        if let Inst::Const { dst, v } = *i
            && defs[dst.0 as usize] == 1
        {
            kc[dst.0 as usize] = Some(v);
        }
    }
    let mut any = false;
    for b in 0..f.blocks.len() {
        let blk = f.blocks.values()[b];
        // block register -> (left, right, descending); length register -> block register
        let mut mk: FxMap<u32, (Reg, Reg, bool)> = FxMap::default();
        let mut len_of: FxMap<u32, u32> = FxMap::default();
        for pc in blk.start as usize..blk.end as usize {
            let i = f.code[pc];
            if let Inst::Cmp { op, dst, a, b: z } = i
                && let Some(&at) = len_of.get(&a.0)
                && let Some(&(l, r, desc)) = mk.get(&at)
                && kc[z.0 as usize] == Some(0)
                && matches!(op, CmpOp::Eq | CmpOp::Ne | CmpOp::Gt | CmpOp::Le)
            {
                // Empty: the bounds are the wrong way round.
                let (x, y) = if desc { (l, r) } else { (r, l) };
                let (cmp, a2, b2) = match op {
                    CmpOp::Eq | CmpOp::Le => (CmpOp::Lt, x, y),
                    _ => (CmpOp::Ge, x, y),
                };
                f.code[pc] = Inst::Cmp {
                    op: cmp,
                    dst,
                    a: a2,
                    b: b2,
                };
                any = true;
            }
            let i = f.code[pc];
            // What this instruction overwrites.
            for_defs(&i, |d| {
                mk.retain(|at, (l, r, _)| *at != d.0 && *l != d && *r != d);
                len_of.retain(|ln, at| *ln != d.0 && *at != d.0);
            });
            match i {
                Inst::MakeDope { at, parts } if parts.len == 3 => {
                    if let Some(&[l, r, dir]) = f.regs(parts)
                        && let Some(dv) = kc[dir.0 as usize]
                        && (dv == 0 || dv == 1)
                        && at != l
                        && at != r
                    {
                        mk.insert(at.0, (l, r, dv == 1));
                    }
                }
                Inst::DopeLen { dst, dope, dim: 0 } if mk.contains_key(&dope.0) && dst != dope => {
                    len_of.insert(dst.0, dope.0);
                }
                _ => {}
            }
        }
    }
    any
}

fn for_uses(f: &crate::Func, i: &Inst, mut g: impl FnMut(Reg)) {
    i.visit(&mut |r, role, _| {
        if role == Role::Use {
            g(r);
        }
    });
    if let Some(l) = i.list() {
        for &r in f.regs(l).unwrap_or(&[]) {
            g(r);
        }
    }
}

/// A branch whose two arms lead to the same block (an arm that is only a jump to the other: the
/// test of a null range, which a local array whose bounds come from a parameter makes before it
/// is made, whichever way it goes) is a jump. The compare that fed it goes with `dead_defs`.
fn equal_arms(f: &mut crate::Func) -> bool {
    let target = |f: &crate::Func, mut b: BlockId| {
        // (A few jump-only blocks in a row; a block that jumps to itself stays.)
        for _ in 0..4 {
            match f
                .blocks
                .get(b)
                .map(|r| &f.code[r.start as usize..r.end as usize])
            {
                Some(&[Inst::Br { to }]) if to != b => b = to,
                _ => break,
            }
        }
        b
    };
    let mut any = false;
    for k in 0..f.blocks.len() {
        let r = f.blocks.values()[k];
        let Some(last) = (r.end as usize).checked_sub(1).filter(|_| r.end > r.start) else {
            continue;
        };
        if let Inst::CondBr { t, f: e, .. } = f.code[last] {
            let (tt, te) = (target(f, t), target(f, e));
            if tt == te {
                f.code[last] = Inst::Br { to: tt };
                any = true;
            }
        }
    }
    any
}

/// Does some jump go to a block `thread` would copy, or some branch to a block that only
/// tests its condition again? (Checked on the finished code before anything is copied.)
fn may_thread(f: &crate::Func) -> bool {
    let nb = f.blocks.len();
    let code = |b: usize| {
        let r = f.blocks.values()[b];
        &f.code[r.start as usize..r.end as usize]
    };
    (0..nb).any(|a| match code(a).last() {
        Some(&Inst::Br { to }) => {
            let l = to.0 as usize;
            l != a
                && l < nb
                && small(code(l))
                && !latch(
                    code(l),
                    (l + 1 < nb).then(|| code(l + 1)),
                    l,
                    f.body.is_some(),
                )
        }
        Some(&Inst::CondBr { t, f: e, .. }) => [t, e]
            .iter()
            .any(|x| (x.0 as usize) < nb && matches!(code(x.0 as usize), [Inst::CondBr { .. }])),
        _ => false,
    })
}

/// Is there a move that follows the instruction computing the value it moves?
fn may_coalesce(f: &crate::Func) -> bool {
    f.blocks.values().iter().any(|b| {
        let code = &f.code[b.start as usize..b.end as usize];
        code.iter().enumerate().any(|(k, i)| {
            let Inst::Move { dst, src } = *i else {
                return false;
            };
            dst != src
                && code[..k].iter().rev().take(16).any(|d| {
                    let mut hit = false;
                    for_defs_fast(d, |r| hit |= r == src);
                    hit && coalescable(d)
                })
        })
    })
}

/// Does some branch go back to a block at or before its own (a loop)?
pub(super) fn has_back_edge(f: &crate::Func) -> bool {
    f.blocks.values().iter().enumerate().any(|(b, r)| {
        f.code[r.end as usize - 1]
            .targets()
            .iter()
            .any(|t| t.0 as usize <= b)
    })
}

/// Run the passes over a finished subprogram that cannot suspend, or a process.
/// Returns whether code was copied into other blocks (the caller walks the result again: what
/// is known on entry to a block decides the branches of its copy).
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    if f.code.len() < 6 {
        return false;
    }
    let mut moved = equal_arms(f);
    // Copying and rewriting pay in loops; most of a library runs a few times, and the
    // optimiser's time counts for every subprogram at its first call.
    let looped = has_back_edge(f);
    if looped && may_thread(f) {
        let c = counts(f);
        moved |= thread(prog, f, &c.0, &c.1);
    }
    if looped
        && f.code.iter().any(|i| matches!(i, Inst::MakeDope { .. }))
        && f.code.iter().any(|i| matches!(i, Inst::DopeLen { .. }))
    {
        let c = counts(f);
        moved |= slice_tests(f, &c.0);
    }
    if moved {
        dead_defs(prog, f);
    }
    if may_coalesce(f) {
        let c = counts(f);
        coalesce(f, &c.0, &c.1);
    }
    moved
}
