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
//! values would stop the fusion), and the temporaries of a linear run (`Bin` add, subtract
//! and multiply, `Pos`, `DopeLen`) defined once, wherever they are read. Classes (`I64`, `F64`, `Ptr`, `Handle`) never mix, so the
//! register types the collector reads stay exact. Hulls are closed, except that a value
//! defined by a simple single-result instruction (`simple`) may take the cell of one whose
//! last read that instruction is.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::*;

const NONE: u32 = u32::MAX;

/// Below this many registers a frame is small and the analysis costs more than it saves
/// (half the functions of a library testbench are below it and hold a seventh of the cells).
const MIN_REGS: usize = 32;

/// What the scan knows of one register. `hi` and `lo` are the hull; the rest says what its
/// ends are made of.
#[derive(Clone, Copy)]
struct Life {
    lo: u32,
    hi: u32,
    def_lo: u32,
    ext_lo: u32,
    /// Last real read, last definition, last end given by liveness (`-1`: none).
    use_hi: i32,
    def_hi: i32,
    ext_hi: i32,
    /// The block of the last definition seen and of the last exposed read recorded.
    defb: u32,
    expb: u32,
    defs: u8,
    uses: u8,
    konst: bool,
}

const NEW: Life = Life {
    lo: NONE,
    hi: 0,
    def_lo: NONE,
    ext_lo: NONE,
    use_hi: -1,
    def_hi: -1,
    ext_hi: -1,
    defb: NONE,
    expb: NONE,
    defs: 0,
    uses: 0,
    konst: false,
};

/// New number of every register (`0`: unused) and the type of every new register.
pub(super) fn plan(
    prog: &Program,
    f: &crate::Func,
    used: &[bool],
) -> Option<(Vec<u32>, Vec<RegTy>)> {
    let n = f.reg_ty.len();
    if used.iter().filter(|&&u| u).count() < MIN_REGS {
        return None;
    }
    let nb = f.blocks.len();
    let mut life = vec![NEW; n];
    // Registers read before written, and registers written, per block (flat, `*_at` offsets).
    let (mut exp, mut exp_at) = (Vec::new(), Vec::with_capacity(nb + 1));
    let (mut dfs, mut dfs_at) = (Vec::new(), Vec::with_capacity(nb + 1));
    let mut buf: Vec<(u32, Role)> = Vec::new();
    for (b, blk) in f.blocks.iter() {
        let b = b.0;
        exp_at.push(exp.len() as u32);
        dfs_at.push(dfs.len() as u32);
        for pc in blk.start..blk.end {
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
                    let l = life.get_mut(r as usize)?;
                    l.lo = l.lo.min(pc);
                    l.hi = l.hi.max(pc);
                    if def {
                        l.def_hi = l.def_hi.max(pc as i32);
                        l.def_lo = l.def_lo.min(pc);
                        l.defb = b;
                        l.defs = l.defs.saturating_add(1);
                        l.konst |= is_const;
                        dfs.push(r);
                    } else {
                        l.uses = l.uses.saturating_add(1);
                        l.use_hi = l.use_hi.max(pc as i32);
                        if l.defb != b && l.expb != b {
                            l.expb = b;
                            exp.push(r);
                        }
                    }
                }
            }
        }
    }
    exp_at.push(exp.len() as u32);
    dfs_at.push(dfs.len() as u32);
    // Liveness of the registers read before they are written in some block.
    if !exp.is_empty() {
        let mut at_start = vec![NONE; f.code.len() + 1];
        for (b, blk) in f.blocks.iter() {
            if blk.end > blk.start && at_start[blk.start as usize] == NONE {
                at_start[blk.start as usize] = b.0;
            }
        }
        let (mut succ, mut succ_at) = (Vec::new(), Vec::with_capacity(nb + 1));
        for (_, blk) in f.blocks.iter() {
            succ_at.push(succ.len() as u32);
            let Some(last) = blk.end.checked_sub(1).filter(|&l| l >= blk.start) else {
                continue;
            };
            match f.code.get(last as usize) {
                Some(Inst::Case { table, .. }) => {
                    if let Some(t) = prog.cases.get(*table) {
                        succ.extend(t.arms.iter().map(|a| a.2.0));
                        succ.push(t.default.0);
                    }
                }
                Some(Inst::Ret { .. } | Inst::Unreachable { .. }) => {}
                Some(i @ (Inst::Br { .. } | Inst::CondBr { .. })) => {
                    succ.extend(i.targets().into_iter().map(|b| b.0));
                }
                _ => succ.extend(Some(at_start[blk.end as usize]).filter(|&b| b != NONE)),
            }
        }
        succ_at.push(succ.len() as u32);
        let mut gidx = vec![NONE; n];
        let mut globals: Vec<u32> = Vec::new();
        for &r in &exp {
            if gidx[r as usize] == NONE {
                gidx[r as usize] = globals.len() as u32;
                globals.push(r);
            }
        }
        let w = globals.len().div_ceil(64);
        if globals.len().saturating_mul(nb) > 1 << 24 {
            return None;
        }
        let mut gen_ = vec![0u64; nb * w];
        let mut kill = vec![0u64; nb * w];
        for b in 0..nb {
            for &r in &exp[exp_at[b] as usize..exp_at[b + 1] as usize] {
                let k = gidx[r as usize] as usize;
                gen_[b * w + k / 64] |= 1 << (k % 64);
            }
            for &r in &dfs[dfs_at[b] as usize..dfs_at[b + 1] as usize] {
                let k = gidx[r as usize];
                if k != NONE {
                    kill[b * w + k as usize / 64] |= 1 << (k % 64);
                }
            }
        }
        let mut live_in = gen_.clone();
        let mut out = vec![0u64; w];
        let succs = |b: usize| &succ[succ_at[b] as usize..succ_at[b + 1] as usize];
        let mut changed = true;
        while changed {
            changed = false;
            for b in (0..nb).rev() {
                out.fill(0);
                for &s in succs(b) {
                    let i = live_in.get(s as usize * w..(s as usize + 1) * w)?;
                    for (o, i) in out.iter_mut().zip(i) {
                        *o |= *i;
                    }
                }
                for x in 0..w {
                    let v = gen_[b * w + x] | (out[x] & !kill[b * w + x]);
                    if v & !live_in[b * w + x] != 0 {
                        live_in[b * w + x] |= v;
                        changed = true;
                    }
                }
            }
        }
        for (b, blk) in f.blocks.iter() {
            let b = b.0 as usize;
            if blk.end <= blk.start {
                continue;
            }
            out.fill(0);
            for &s in succs(b) {
                for (o, i) in out.iter_mut().zip(&live_in[s as usize * w..]) {
                    *o |= *i;
                }
            }
            for x in 0..w {
                let mut bits = live_in[b * w + x];
                while bits != 0 {
                    let l = &mut life[globals[x * 64 + bits.trailing_zeros() as usize] as usize];
                    bits &= bits - 1;
                    l.lo = l.lo.min(blk.start);
                    l.ext_lo = l.ext_lo.min(blk.start);
                }
                let mut bits = out[x];
                while bits != 0 {
                    let l = &mut life[globals[x * 64 + bits.trailing_zeros() as usize] as usize];
                    bits &= bits - 1;
                    l.hi = l.hi.max(blk.end - 1);
                    l.ext_hi = l.ext_hi.max(blk.end as i32 - 1);
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
        let drop = d.is_some_and(|k| life[k].konst && life[k].defs == 1);
        loose[pc + 1] = loose[pc] + u32::from(drop);
    }
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
                | Inst::Select { .. }
                | Inst::Load { .. }
                | Inst::Index { .. }
                | Inst::Pos { .. }
                | Inst::SlicePos { .. }
                | Inst::DopeLen { .. }
                | Inst::Field { .. }
        )
    };
    // Linear scan.
    let mut order: Vec<u32> = (0..n as u32)
        .filter(|&r| {
            used[r as usize]
                && life[r as usize].lo != NONE
                && (r == 0 || f.reg_ty[r as usize - 1] != RegTy::Sig)
        })
        .collect();
    order.sort_unstable_by_key(|&r| (life[r as usize].lo, r));
    let mut map = vec![0u32; n];
    let mut ty: Vec<RegTy> = Vec::new();
    let class = |t: RegTy| match t {
        RegTy::I64 => Some(0),
        RegTy::F64 => Some(1),
        RegTy::Ptr => Some(2),
        RegTy::Handle => Some(3),
        RegTy::Sig => None,
    };
    let mut active: [BinaryHeap<Reverse<(u32, u8, u32)>>; 4] = Default::default();
    let mut free: [Vec<u32>; 4] = Default::default();
    for r in order {
        let k = r as usize;
        let (t, l) = (f.reg_ty[k], life[k]);
        let fused = l.defs == 1
            && fuses(&f.code[l.lo as usize])
            && l.uses == 1
            && l.hi > l.lo
            && (loose[l.hi as usize] - loose[l.lo as usize + 1]) == l.hi - l.lo - 1;
        // The temporaries of a linear run (`Bin`, `Pos`, `DopeLen`), wherever they are read:
        // the predecoder folds the run into one op per result something else reads, which it
        // can do only for registers defined once (#534).
        let linear = l.defs == 1
            && l.uses >= 1
            && matches!(
                f.code[l.lo as usize],
                Inst::Bin {
                    op: IntOp::Add | IntOp::Sub | IntOp::Mul,
                    ..
                } | Inst::Pos { dim: 0, .. }
                    | Inst::DopeLen { dim: 0, .. }
            );
        let pinned = (l.konst && l.defs == 1) || fused || linear;
        match class(t).filter(|_| !pinned) {
            None => {
                map[k] = ty.len() as u32;
                ty.push(t);
                if t == RegTy::Sig {
                    ty.push(*f.reg_ty.get(k + 1)?);
                }
            }
            Some(c) => {
                let takes = l.defs == 1 && l.def_lo == l.lo && l.ext_lo > l.lo && simple(l.lo);
                while let Some(&Reverse((h, rw, cell))) = active[c].peek() {
                    if h > l.lo || (h == l.lo && !(takes && rw == 0)) {
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
                // Is `hi` the last real read, with nothing else touching the cell there?
                let ro = l.hi as i32 == l.use_hi && l.ext_hi < l.use_hi && l.def_hi < l.use_hi;
                active[c].push(Reverse((l.hi, u8::from(!ro), cell)));
            }
        }
    }
    Some((map, ty))
}
