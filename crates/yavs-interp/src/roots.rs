//! Precise register roots for a collection inside an activation (plan rule 6, #422): which
//! `Handle` and `Ptr` registers are live at each safepoint of a function, from a liveness
//! analysis of its code, computed when a collection first meets the function.
//!
//! A frame can be at a safepoint when a collection runs: the top frame at a `new`, a
//! `deallocate` or a native call (`Machine::reclaim`), every frame below it at the call it
//! waits in. A `Handle` register live there is a root; a `Ptr` register live there keeps the
//! heap object it points into.

use yavs_ir::{Func, Inst, Program, Reg, RegTy, Role};

/// The `Handle` and `Ptr` registers live at each safepoint of one function.
#[derive(Debug, Default)]
pub(crate) struct LiveRefs {
    /// `(pc, start, len)` into `regs`, sorted by pc.
    at: Vec<(u32, u32, u32)>,
    regs: Vec<Reg>,
}

impl LiveRefs {
    /// The registers live at `pc`; `None` when `pc` is not a safepoint.
    pub fn get(&self, pc: u32) -> Option<&[Reg]> {
        let k = self.at.binary_search_by_key(&pc, |e| e.0).ok()?;
        let (_, s, n) = self.at[k];
        self.regs.get(s as usize..(s + n) as usize)
    }
}

/// Where a collection can find this frame.
fn is_safepoint(i: &Inst) -> bool {
    matches!(
        i,
        Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::Free { .. }
            | Inst::CallNative { .. }
            | Inst::Call { .. }
            | Inst::CallIndirect { .. }
            | Inst::CallProt { .. }
            | Inst::CallForeign { .. }
    )
}

/// A bit set over dense register indices.
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        self.0[i as usize / 64] |= 1 << (i % 64);
    }
    /// The set bits, ascending.
    fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(w, &x)| {
            let mut x = x;
            std::iter::from_fn(move || {
                (x != 0).then(|| {
                    let b = x.trailing_zeros();
                    x &= x - 1;
                    w as u32 * 64 + b
                })
            })
        })
    }
}

/// The live set of one backward walk over a block: bits plus the words it dirtied, so it is
/// cleared in time proportional to what the block touched (a function can have tens of
/// thousands of registers and thousands of blocks).
struct Walk {
    bits: Bits,
    dirty: Vec<usize>,
}

impl Walk {
    fn add(&mut self, i: u32) {
        self.bits.set(i);
        self.dirty.push(i as usize / 64);
    }
    fn remove(&mut self, i: u32) {
        self.bits.0[i as usize / 64] &= !(1 << (i % 64));
    }
    fn clear(&mut self) {
        for w in self.dirty.drain(..) {
            self.bits.0[w] = 0;
        }
    }
    /// The set bits, ascending (only the dirtied words can hold any).
    fn ones(&mut self) -> Vec<u32> {
        self.dirty.sort_unstable();
        self.dirty.dedup();
        let mut v = Vec::new();
        for &w in &self.dirty {
            let mut x = self.bits.0[w];
            while x != 0 {
                v.push(w as u32 * 64 + x.trailing_zeros());
                x &= x - 1;
            }
        }
        v
    }
}

/// With `all` (the `gc-stress` feature) every register is tracked, so a collection can check
/// that no other live register holds a handle.
pub(crate) fn live_refs(prog: &Program, f: &Func, all: bool) -> LiveRefs {
    // Dense indices of the tracked registers.
    let mut idx = vec![u32::MAX; f.reg_ty.len()];
    let mut tracked = Vec::new();
    for (r, t) in f.reg_ty.iter().enumerate() {
        if all || matches!(t, RegTy::Handle | RegTy::Ptr) {
            idx[r] = tracked.len() as u32;
            tracked.push(Reg(r as u32));
        }
    }
    let n = tracked.len();
    let id = |r: Reg| idx.get(r.0 as usize).copied().filter(|&i| i != u32::MAX);
    // The tracked registers an instruction defines and uses.
    let ops = |i: &Inst, defs: &mut Vec<u32>, uses: &mut Vec<u32>| {
        defs.clear();
        uses.clear();
        i.visit(&mut |r, role, _| match (role, id(r)) {
            (Role::Def, Some(k)) => defs.push(k),
            (Role::Use, Some(k)) => uses.push(k),
            _ => {}
        });
        if let Some(rs) = i.list().and_then(|l| f.regs(l)) {
            uses.extend(rs.iter().filter_map(|&r| id(r)));
        }
    };
    let (mut defs, mut uses) = (Vec::new(), Vec::new());
    // live = (live - defs) | uses, backwards over one instruction.
    let mut step = |live: &mut Walk, i: &Inst| {
        ops(i, &mut defs, &mut uses);
        for &k in &defs {
            live.remove(k);
        }
        for &k in &uses {
            live.add(k);
        }
    };
    let blocks: Vec<(u32, u32)> = f.blocks.iter().map(|(_, b)| (b.start, b.end)).collect();
    let succs: Vec<Vec<usize>> = blocks
        .iter()
        .map(|&(s, e)| {
            let next = |pc: u32| blocks.iter().position(|b| b.0 == pc && b.1 > b.0);
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
                _ => next(e).into_iter().collect(),
            }
        })
        .collect();
    let mut walk = Walk {
        bits: Bits::new(n),
        dirty: Vec::new(),
    };
    // Registers live into each block on their own (used before any definition in it); only
    // those take part in the dataflow, the others live within one block.
    let mut exposed: Vec<Vec<u32>> = Vec::with_capacity(blocks.len());
    for &(s, e) in &blocks {
        for pc in (s..e).rev() {
            step(&mut walk, &f.code[pc as usize]);
        }
        exposed.push(walk.ones());
        walk.clear();
    }
    let mut gidx = vec![u32::MAX; n];
    let mut globals = Vec::new();
    for &k in exposed.iter().flatten() {
        if gidx[k as usize] == u32::MAX {
            gidx[k as usize] = globals.len() as u32;
            globals.push(k);
        }
    }
    let g = globals.len();
    // Per block over the global registers: `gen` (live into it on its own) and `kill`
    // (defined in it); live-in = gen | (live-out - kill).
    let mut gen_: Vec<Bits> = Vec::with_capacity(blocks.len());
    let mut kill: Vec<Bits> = Vec::with_capacity(blocks.len());
    let (mut kd, mut ku) = (Vec::new(), Vec::new());
    for (b, &(s, e)) in blocks.iter().enumerate() {
        let mut gb = Bits::new(g);
        for &k in &exposed[b] {
            gb.set(gidx[k as usize]);
        }
        let mut kb = Bits::new(g);
        for pc in s..e {
            ops(&f.code[pc as usize], &mut kd, &mut ku);
            for &k in &kd {
                if gidx[k as usize] != u32::MAX {
                    kb.set(gidx[k as usize]);
                }
            }
        }
        gen_.push(gb);
        kill.push(kb);
    }
    let words = g.div_ceil(64);
    let mut live_in: Vec<Bits> = (0..blocks.len()).map(|_| Bits::new(g)).collect();
    let mut out = Bits::new(g);
    let mut changed = g > 0;
    while changed {
        changed = false;
        for b in (0..blocks.len()).rev() {
            out.0.fill(0);
            for &s in &succs[b] {
                if let Some(i) = live_in.get(s) {
                    for w in 0..words {
                        out.0[w] |= i.0[w];
                    }
                }
            }
            for w in 0..words {
                let v = gen_[b].0[w] | (out.0[w] & !kill[b].0[w]);
                if v & !live_in[b].0[w] != 0 {
                    live_in[b].0[w] |= v;
                    changed = true;
                }
            }
        }
    }
    let mut res = LiveRefs::default();
    for (b, &(s, e)) in blocks.iter().enumerate() {
        for &sb in &succs[b] {
            if let Some(i) = live_in.get(sb) {
                for j in i.ones() {
                    walk.add(globals[j as usize]);
                }
            }
        }
        let mut here = Vec::new();
        for pc in (s..e).rev() {
            let i = &f.code[pc as usize];
            step(&mut walk, i);
            if is_safepoint(i) {
                let live = walk.ones();
                here.push((pc, live.into_iter().map(|k| tracked[k as usize])));
            }
        }
        walk.clear();
        for (pc, regs) in here.into_iter().rev() {
            let start = res.regs.len() as u32;
            res.regs.extend(regs);
            res.at.push((pc, start, res.regs.len() as u32 - start));
        }
    }
    res.at.sort_unstable_by_key(|e| e.0);
    res
}
