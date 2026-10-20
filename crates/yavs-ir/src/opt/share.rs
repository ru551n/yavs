//! Register sharing (#478): registers whose live ranges do not overlap get one cell.
//!
//! Runs last, over the finished code: a backward liveness analysis over the blocks gives
//! every register a hull `[lo, hi]` of instruction indices (first definition or block it is
//! live into, to last use or block it is live out of); a linear scan over the hulls, in
//! order of `lo`, hands out cells per register class. A frame then has one cell per
//! simultaneously live value instead of one per value.
//!
//! What is never shared: a signal reference (two adjacent cells), a register the
//! interpreter preloads (defined once by a constant or an address: frames start with it and
//! it may be read where its definition has not run), and a compare, index, position, layout
//! or signal-value temporary defined once and read once by a following instruction (the
//! predecoder fuses such pairs by counting a register's reads, so a cell read by several
//! values would stop the fusion). Classes (`I64`, `F64`, `Ptr`, `Handle`) never mix, so the
//! register types the collector reads stay exact. Hulls are closed, except that a value
//! defined by a simple single-result instruction (`simple`) may take the cell of one whose
//! last read that instruction is.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::*;

type Bits = Vec<u64>;

/// New number of every register (`u32::MAX`: unused) and the type of every new register.
pub(super) fn plan(
    prog: &Program,
    f: &crate::Func,
    used: &[bool],
) -> Option<(Vec<u32>, Vec<RegTy>)> {
    let n = f.reg_ty.len();
    let blocks: Vec<(u32, u32)> = f.blocks.iter().map(|(_, b)| (b.start, b.end)).collect();
    let nb = blocks.len();
    let none = u32::MAX;
    let (mut lo, mut hi) = (vec![none; n], vec![0u32; n]);
    let (mut defs, mut uses) = (vec![0u8; n], vec![0u8; n]);
    let mut konst = vec![false; n];
    // Last real use, last definition, first definition; the ends the liveness analysis added.
    let (mut use_hi, mut def_hi) = (vec![-1i64; n], vec![-1i64; n]);
    let (mut def_lo, mut ext_lo) = (vec![none; n], vec![none; n]);
    let mut ext_hi = vec![-1i64; n];
    let (mut defb, mut expb) = (vec![none; n], vec![none; n]);
    let mut exposed: Vec<Vec<u32>> = vec![Vec::new(); nb];
    let mut buf: Vec<(u32, Role)> = Vec::new();
    for (b, &(s, e)) in blocks.iter().enumerate() {
        for pc in s..e {
            let i = f.code.get(pc as usize)?;
            buf.clear();
            i.visit(&mut |r, role, _| buf.push((r.0, role)));
            if let Some(l) = i.list() {
                buf.extend(f.regs(l)?.iter().map(|r| (r.0, Role::Use)));
            }
            let is_const = matches!(
                i,
                Inst::Const { .. }
                    | Inst::ConstF { .. }
                    | Inst::AddrOf {
                        space: AddrSpace::Frame
                            | AddrSpace::Inst
                            | AddrSpace::Pkg(_)
                            | AddrSpace::Const(_)
                            | AddrSpace::Shared,
                        ..
                    }
            );
            for def in [false, true] {
                for &(r, role) in &buf {
                    if (role == Role::Def) != def {
                        continue;
                    }
                    let k = r as usize;
                    if k >= n {
                        return None;
                    }
                    lo[k] = lo[k].min(pc);
                    hi[k] = hi[k].max(pc);
                    if def {
                        def_hi[k] = def_hi[k].max(i64::from(pc));
                        def_lo[k] = def_lo[k].min(pc);
                        defb[k] = b as u32;
                        defs[k] = defs[k].saturating_add(1);
                        konst[k] |= is_const;
                    } else {
                        uses[k] = uses[k].saturating_add(1);
                        use_hi[k] = use_hi[k].max(i64::from(pc));
                        if defb[k] != b as u32 && expb[k] != b as u32 {
                            expb[k] = b as u32;
                            exposed[b].push(r);
                        }
                    }
                }
            }
        }
    }
    // Successors of every block.
    let mut at_start = vec![none; f.code.len() + 1];
    for (b, &(s, e)) in blocks.iter().enumerate().rev() {
        if e > s {
            at_start[s as usize] = b as u32;
        }
    }
    let succs: Vec<Vec<usize>> = blocks
        .iter()
        .map(|&(s, e)| {
            let Some(last) = e.checked_sub(1).filter(|&l| l >= s) else {
                return Vec::new();
            };
            match f.code.get(last as usize) {
                Some(Inst::Case { table, .. }) => prog
                    .cases
                    .get(*table)
                    .map(|t| {
                        t.arms
                            .iter()
                            .map(|a| a.2)
                            .chain([t.default])
                            .map(|b| b.0 as usize)
                            .collect()
                    })
                    .unwrap_or_default(),
                Some(Inst::Ret { .. } | Inst::Unreachable { .. }) => Vec::new(),
                Some(i @ (Inst::Br { .. } | Inst::CondBr { .. })) => {
                    i.targets().into_iter().map(|b| b.0 as usize).collect()
                }
                _ => at_start
                    .get(e as usize)
                    .filter(|&&b| b != none)
                    .map(|&b| b as usize)
                    .into_iter()
                    .collect(),
            }
        })
        .collect();
    // Liveness of the registers read before they are written in some block.
    let mut gidx = vec![none; n];
    let mut globals: Vec<u32> = Vec::new();
    for &r in exposed.iter().flatten() {
        if gidx[r as usize] == none {
            gidx[r as usize] = globals.len() as u32;
            globals.push(r);
        }
    }
    let g = globals.len();
    let w = g.div_ceil(64);
    if g.saturating_mul(nb) > 1 << 27 {
        return None;
    }
    let mut gen_: Vec<Bits> = vec![vec![0; w]; nb];
    let mut kill: Vec<Bits> = vec![vec![0; w]; nb];
    for (b, &(s, e)) in blocks.iter().enumerate() {
        for &r in &exposed[b] {
            let k = gidx[r as usize] as usize;
            gen_[b][k / 64] |= 1 << (k % 64);
        }
        if g == 0 {
            continue;
        }
        for pc in s..e {
            f.code[pc as usize].visit(&mut |r, role, _| {
                let k = gidx.get(r.0 as usize).copied().unwrap_or(none);
                if role == Role::Def && k != none {
                    kill[b][k as usize / 64] |= 1 << (k % 64);
                }
            });
        }
    }
    let mut live_in = gen_.clone();
    let mut out = vec![0u64; w];
    let mut changed = g > 0;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            out.fill(0);
            for &s in &succs[b] {
                for (o, i) in out.iter_mut().zip(live_in.get(s)?) {
                    *o |= *i;
                }
            }
            for x in 0..w {
                let v = gen_[b][x] | (out[x] & !kill[b][x]);
                if v & !live_in[b][x] != 0 {
                    live_in[b][x] |= v;
                    changed = true;
                }
            }
        }
    }
    for (b, &(s, e)) in blocks.iter().enumerate() {
        if e <= s {
            continue;
        }
        out.fill(0);
        for &sb in &succs[b] {
            for (o, i) in out.iter_mut().zip(live_in.get(sb)?) {
                *o |= *i;
            }
        }
        for x in 0..w {
            for (bits, is_in) in [(live_in[b][x], true), (out[x], false)] {
                let mut bits = bits;
                while bits != 0 {
                    let r = globals[x * 64 + bits.trailing_zeros() as usize] as usize;
                    bits &= bits - 1;
                    if is_in {
                        lo[r] = lo[r].min(s);
                        ext_lo[r] = ext_lo[r].min(s);
                    } else {
                        hi[r] = hi[r].max(e - 1);
                        ext_hi[r] = ext_hi[r].max(i64::from(e - 1));
                    }
                }
            }
        }
    }
    // Instructions the predecoder drops (preloaded definitions): a pair fuses across them.
    let mut loose = vec![0u32; f.code.len() + 1];
    for (pc, i) in f.code.iter().enumerate() {
        let mut d = None;
        if matches!(
            i,
            Inst::Const { .. } | Inst::ConstF { .. } | Inst::AddrOf { .. }
        ) {
            i.visit(&mut |r, role, _| {
                if role == Role::Def {
                    d = Some(r.0 as usize);
                }
            });
        }
        let drop = d.is_some_and(|k| konst[k] && defs[k] == 1);
        loose[pc + 1] = loose[pc] + u32::from(drop);
    }
    // Linear scan.
    let mut order: Vec<u32> = (0..n as u32)
        .filter(|&r| {
            used[r as usize]
                && lo[r as usize] != none
                && (r == 0 || f.reg_ty[r as usize - 1] != RegTy::Sig)
        })
        .collect();
    order.sort_by_key(|&r| (lo[r as usize], r));
    let mut map = vec![none; n];
    let mut ty: Vec<RegTy> = Vec::new();
    let class = |t: RegTy| match t {
        RegTy::I64 => Some(0),
        RegTy::F64 => Some(1),
        RegTy::Ptr => Some(2),
        RegTy::Handle => Some(3),
        RegTy::Sig => None,
    };
    // The producers the predecoder fuses with their reader (`pre.rs`).
    let fuses = |i: &Inst| {
        matches!(
            i,
            Inst::Cmp { .. }
                | Inst::Index { .. }
                | Inst::Pos { .. }
                | Inst::LayoutOf { .. }
                | Inst::SigVal { .. }
        )
    };
    // A value defined by a simple instruction may take the cell of one whose last read it is.
    let simple = |pc: u32| {
        matches!(
            f.code[pc as usize],
            Inst::Bin { .. }
                | Inst::Un { .. }
                | Inst::Cmp { .. }
                | Inst::Bool { .. }
                | Inst::Move { .. }
                | Inst::Load { .. }
                | Inst::Index { .. }
                | Inst::Pos { .. }
                | Inst::DopeLen { .. }
                | Inst::Field { .. }
        )
    };
    let mut active: [BinaryHeap<Reverse<(u32, u8, u32)>>; 4] = Default::default();
    let mut free: [Vec<u32>; 4] = Default::default();
    for r in order {
        let k = r as usize;
        let t = f.reg_ty[k];
        let fused = defs[k] == 1
            && fuses(&f.code[lo[k] as usize])
            && uses[k] == 1
            && hi[k] > lo[k]
            && (loose[hi[k] as usize] - loose[lo[k] as usize + 1]) == hi[k] - lo[k] - 1;
        let pinned = (konst[k] && defs[k] == 1) || fused;
        match class(t).filter(|_| !pinned) {
            None => {
                map[k] = ty.len() as u32;
                ty.push(t);
                if t == RegTy::Sig {
                    ty.push(*f.reg_ty.get(k + 1)?);
                }
            }
            Some(c) => {
                let takes =
                    defs[k] == 1 && def_lo[k] == lo[k] && ext_lo[k] > lo[k] && simple(lo[k]);
                while let Some(&Reverse((h, rw, cell))) = active[c].peek() {
                    if h > lo[k] || (h == lo[k] && !(takes && rw == 0)) {
                        break;
                    }
                    active[c].pop();
                    free[c].push(cell);
                }
                let cell = free[c].pop().unwrap_or_else(|| {
                    ty.push(t);
                    ty.len() as u32 - 1
                });
                map[k] = cell;
                // Reads last at `hi` and nothing else touches it there?
                let ro =
                    i64::from(hi[k]) == use_hi[k] && ext_hi[k] < use_hi[k] && def_hi[k] < use_hi[k];
                active[c].push(Reverse((hi[k], u8::from(!ro), cell)));
            }
        }
    }
    for m in &mut map {
        if *m == none {
            *m = 0;
        }
    }
    Some((map, ty))
}
