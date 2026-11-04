//! A logic row or slice read through a temporary copy is read in place:
//! `Alloc t; LCopy t, 0, src, so, n; ...; LGet x, t, i` is `LGet x, src, so + i`, and
//! `...; LCopy d, do, t, to, m` is `LCopy d, do, src, so + to, m` (`LCopy` reads before it writes).
//!
//! A value has no plane offset, so the lowering copies a row (`hconfig(j)`) or a slice into a
//! temporary and reads that. The temporary goes when nothing can tell: `t` is read by nothing
//! else, every reader is after the copy in its basic block, nothing between can write what `src`
//! names, `src` and `so` keep their values and the reader stays inside the copied run. No check
//! moves or goes. Rules and what still copies: `docs/decisions.md` "Logic rows read in place".

use super::*;
use crate::Regs;

impl Opt<'_> {
    /// The constant a register defined once holds.
    fn place_k(&self, r: Reg) -> Option<i64> {
        let r = self.resolve(r);
        if self.defs.get(r.0 as usize) != Some(&1) {
            return None;
        }
        self.kc.get(r.0 as usize).copied().flatten()
    }

    /// The instruction that defines the register, when it is defined once.
    fn place_def(&self, r: Reg) -> Option<Inst> {
        if !self.single(r) {
            return None;
        }
        let i = *self.code.get(*self.def_pc.get(r.0 as usize)? as usize)?;
        let mut defines = false;
        i.visit(&mut |x, role, _| defines |= role == Role::Def && x == r);
        defines.then_some(i)
    }

    /// Is `i` certainly an element below `n`: constants, or `n - 1 - pos` with `pos` a checked
    /// position in the descriptor of `n` elements.
    // ponytail: a loop variable as the position keeps the copy; interval facts could prove it.
    fn place_below(&self, i: Reg, n: Reg) -> bool {
        let kn = self.place_k(n);
        if let (Some(ki), Some(kn)) = (self.place_k(i), kn) {
            return 0 <= ki && ki < kn;
        }
        let Some(Inst::Bin {
            op: IntOp::Sub,
            a: last,
            b: p,
            ..
        }) = self.place_def(i)
        else {
            return false;
        };
        // (A repeated position is a `Move` of the first.)
        let mut pos = self.place_def(p);
        while let Some(Inst::Move { src, .. }) = pos {
            pos = self.place_def(src);
        }
        let Some(Inst::Pos { dope, dim: 0, .. }) = pos else {
            return false;
        };
        let last_ok = match self.place_def(last) {
            Some(Inst::Bin {
                op: IntOp::Sub,
                a,
                b,
                ..
            }) => a == n && self.place_k(b) == Some(1),
            _ => kn.is_some() && self.place_k(last) == kn.map(|k| k - 1),
        };
        last_ok && self.place_len_of(dope, n)
    }

    /// Is the run of `m` elements from `off` certainly inside `n`: constants, or all of them, the
    /// lengths equal or checked equal by a `SubtypeCheck` between `pc` and `q`.
    fn place_run_in(&self, off: Reg, m: Reg, n: Reg, pc: usize, q: usize) -> bool {
        let (ko, km, kn) = (self.place_k(off), self.place_k(m), self.place_k(n));
        if let (Some(a), Some(c), Some(k)) = (ko, km, kn) {
            return 0 <= a && 0 <= c && a.checked_add(c).is_some_and(|e| e <= k);
        }
        if ko != Some(0) {
            return false;
        }
        m == n
            || matches!(self.place_def(m), Some(Inst::DopeLen { dope, dim: 0, .. })
                if self.place_len_of(dope, n))
            || self.code[pc + 1..q].iter().any(|i| {
                matches!(*i, Inst::SubtypeCheck { dope, want, dims: 1, .. }
                    if self.place_len_of(dope, n) && self.place_len_of(want, m)
                        || self.place_len_of(dope, m) && self.place_len_of(want, n))
            })
    }

    /// Does the descriptor `dope` have `n` elements in its first dimension?
    fn place_len_of(&self, dope: Reg, n: Reg) -> bool {
        if let Some(Inst::DopeLen {
            dope: d, dim: 0, ..
        }) = self.place_def(n)
            && d == dope
        {
            return true;
        }
        self.single(dope)
            && self.place_k(n).is_some_and(|k| {
                self.const_dim(self.opn(dope), 0)
                    .and_then(|(l, r, dir)| dim_len(l, r, dir == 1))
                    == u64::try_from(k).ok()
            })
    }

    /// Read rows and slices of logic arrays where they are; whether any copy went.
    pub(super) fn forward_logic_copies(&mut self) -> bool {
        // Who reads the destination of each copy into a block defined once.
        let mut readers: FxMap<u32, Vec<u32>> = FxMap::default();
        let mut alloc: FxMap<u32, u32> = FxMap::default();
        for (pc, i) in self.code.iter().enumerate() {
            if let Inst::Alloc { dst, .. } = *i
                && self.single(dst)
            {
                alloc.insert(dst.0, pc as u32);
            }
        }
        if alloc.is_empty() {
            return false;
        }
        for (pc, i) in self.code.iter().enumerate() {
            if is_dead(i) {
                continue;
            }
            self.place_uses(i, |r| {
                if alloc.contains_key(&r.0) {
                    readers.entry(r.0).or_default().push(pc as u32);
                }
            });
        }
        for (_, i) in &self.inserts {
            self.place_uses(i, |r| {
                alloc.remove(&r.0);
            });
        }
        let mut done = false;
        for b in 0..self.blocks.len() {
            for pc in self.range(b) {
                let Inst::LCopy {
                    dst: t,
                    dst_off,
                    src,
                    src_off,
                    n,
                } = self.code[pc]
                else {
                    continue;
                };
                if !alloc.contains_key(&t.0) || self.place_k(dst_off) != Some(0) || src == t {
                    continue;
                }
                let Some(rs) = readers.get(&t.0).filter(|rs| rs.len() >= 2) else {
                    continue;
                };
                let rest: Vec<usize> = rs
                    .iter()
                    .map(|&p| p as usize)
                    .filter(|&p| p != pc)
                    .collect();
                if rest.len() + 1 != rs.len() || !self.place_can(b, pc, t, src, src_off, n, &rest) {
                    continue;
                }
                for &q in &rest {
                    self.place_rewrite(b, q, t, src, src_off);
                }
                self.code[pc] = dead();
                self.changed = true;
                done = true;
            }
        }
        done
    }

    fn place_uses(&self, i: &Inst, mut f: impl FnMut(Reg)) {
        i.visit(&mut |r, role, _| {
            if role == Role::Use {
                f(r);
            }
        });
        if let Some(l) = i.list() {
            for &r in self
                .operands
                .get(l.start as usize..(l.start + l.len) as usize)
                .unwrap_or(&[])
            {
                f(r);
            }
        }
    }

    /// May the readers `rest` of block `t`, filled by the `LCopy` at `pc`, read `src` instead?
    #[allow(clippy::too_many_arguments)]
    fn place_can(
        &self,
        b: usize,
        pc: usize,
        t: Reg,
        src: Reg,
        so: Reg,
        n: Reg,
        rest: &[usize],
    ) -> bool {
        let block = self.range(b);
        let Some(&last) = rest.iter().max() else {
            return false;
        };
        for &q in rest {
            if q <= pc || !block.contains(&q) {
                return false;
            }
            let ok = match self.code[q] {
                Inst::LGet { base, idx, .. } => base == t && idx != t && self.place_below(idx, n),
                Inst::LCopy {
                    dst,
                    dst_off,
                    src: s,
                    src_off,
                    n: m,
                } => {
                    s == t
                        && dst != t
                        && dst_off != t
                        && src_off != t
                        && m != t
                        && self.place_run_in(src_off, m, n, pc, q)
                }
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        // Between the copy and the last reader: nothing that may write what `src` names and no
        // new value of `src` or `so`. Blocks made after the copy are fresh memory.
        let mut fresh: Vec<Reg> = Vec::new();
        for q in pc + 1..last {
            let i = self.code[q];
            if is_dead(&i) {
                continue;
            }
            let safe = match i {
                Inst::Const { .. }
                | Inst::ConstF { .. }
                | Inst::Move { .. }
                | Inst::Select { .. }
                | Inst::AddrOf { .. }
                | Inst::Load { .. }
                | Inst::LGet { .. }
                | Inst::Field { .. }
                | Inst::LayoutOf { .. }
                | Inst::Bin { .. }
                | Inst::BinRange { .. }
                | Inst::Un { .. }
                | Inst::Cmp { .. }
                | Inst::Bool { .. }
                | Inst::Check { .. }
                | Inst::Pos { .. }
                | Inst::SlicePos { .. }
                | Inst::DopeLen { .. }
                | Inst::BoundsLen { .. }
                | Inst::SubtypeCheck { .. }
                | Inst::Deref { .. } => true,
                Inst::Alloc { dst, .. } => {
                    if self.single(dst) {
                        fresh.push(dst);
                    }
                    true
                }
                Inst::Index { dst, base, .. } => {
                    if fresh.contains(&base) && self.single(dst) {
                        fresh.push(dst);
                    }
                    true
                }
                Inst::MakeDope { at, .. } => fresh.contains(&at),
                Inst::Store { addr, .. } => fresh.contains(&addr),
                Inst::LCopy { dst, .. } | Inst::Copy { dst, .. } => fresh.contains(&dst),
                Inst::LSet { base, .. } | Inst::LFill { base, .. } => fresh.contains(&base),
                _ => false,
            };
            if !safe {
                return false;
            }
            let mut moved = false;
            i.visit(&mut |r, role, _| moved |= role == Role::Def && (r == src || r == so));
            if moved {
                return false;
            }
        }
        true
    }

    /// The reader at `q` reads `src` from element `so` on instead of block `t`.
    fn place_rewrite(&mut self, b: usize, q: usize, t: Reg, src: Reg, so: Reg) {
        let off = |o: &mut Self, at: Reg| -> Reg {
            match (o.place_k(so), o.place_k(at)) {
                (Some(0), _) => at,
                (_, Some(0)) => so,
                (Some(x), Some(y)) => o.const_reg(b, q, x + y),
                _ => {
                    let dst = o.fresh(RegTy::I64, 1);
                    o.inserts.push((
                        q,
                        Inst::Bin {
                            op: IntOp::Add,
                            dst,
                            a: so,
                            b: at,
                        },
                    ));
                    dst
                }
            }
        };
        match self.code[q] {
            Inst::LGet { dst, base, idx } if base == t => {
                let idx = off(self, idx);
                self.code[q] = Inst::LGet {
                    dst,
                    base: src,
                    idx,
                };
            }
            Inst::LCopy {
                dst,
                dst_off,
                src: s,
                src_off,
                n,
            } if s == t => {
                let src_off = off(self, src_off);
                self.code[q] = Inst::LCopy {
                    dst,
                    dst_off,
                    src,
                    src_off,
                    n,
                };
            }
            _ => {}
        }
    }
}

// A one-element signal read that only a concatenation takes is a scalar read: `Alloc t; SigRead t, s; ...;
// Cat d, [.., t, 1, ..]` is `SigReadScalar x, s; ...; Cat d, [.., x, ..]` with that piece a scalar. The block and
// the vector read go. Only a logic `Cat` of one unit per element and no bias (`std_ulogic` arrays; `bit` pieces
// are positions, not codes), and only a block nothing else reads.
impl Opt<'_> {
    /// Replace such reads; whether any went.
    pub(super) fn cat_reads(&mut self) -> bool {
        // The reads into a block defined once by an `Alloc`.
        let mut reads: Vec<(usize, Reg)> = Vec::new();
        for (pc, i) in self.code.iter().enumerate() {
            if let Inst::SigRead { dst, .. } = *i
                && matches!(self.place_def(dst), Some(Inst::Alloc { .. }))
            {
                reads.push((pc, dst));
            }
        }
        if reads.is_empty() {
            return false;
        }
        let mut users: FxMap<u32, Vec<usize>> = FxMap::default();
        for &(_, t) in &reads {
            users.insert(t.0, Vec::new());
        }
        for (pc, i) in self.code.iter().enumerate() {
            if !is_dead(i) {
                self.place_uses(i, |r| {
                    if let Some(u) = users.get_mut(&r.0) {
                        u.push(pc);
                    }
                });
            }
        }
        for (_, i) in &self.inserts {
            self.place_uses(i, |r| {
                users.remove(&r.0);
            });
        }
        let mut done = false;
        for (pc, t) in reads {
            let Some(us) = users.get(&t.0) else { continue };
            let Inst::SigRead { sig, .. } = self.code[pc] else {
                continue;
            };
            // Its uses: this read and one `Cat`, with the `Alloc` all in one block.
            let z = self.def_pc[t.0 as usize] as usize;
            let Some(q) = us
                .iter()
                .copied()
                .find(|&u| matches!(self.code[u], Inst::Cat { .. }))
            else {
                continue;
            };
            if us.len() != 2 || !(z < pc && pc < q) || !self.same_block(z, q) {
                continue;
            }
            let Inst::Cat {
                dst,
                n,
                m,
                parts,
                scalars,
                bias: 0,
                logic: true,
                w,
            } = self.code[q]
            else {
                continue;
            };
            let list = self
                .operands
                .get(parts.start as usize..(parts.start + parts.len) as usize)
                .map(<[Reg]>::to_vec)
                .unwrap_or_default();
            let Some(k) = list.iter().step_by(2).position(|&r| r == t) else {
                continue;
            };
            let one = |s: &Self, r: Reg| s.place_k(r) == Some(1);
            // (A signal reference is no plain value: `place_def` does not take it.)
            let sig_one = self.defs.get(sig.0 as usize) == Some(&1)
                && matches!(self.code.get(self.def_pc[sig.0 as usize] as usize),
                    Some(Inst::SigVal { dst, len, .. }) if *dst == sig && one(self, *len));
            if scalars >> k & 1 != 0 || !one(self, m) || !one(self, list[2 * k + 1]) || !sig_one {
                continue;
            }
            let raw = self.fresh(RegTy::I64, 1);
            let mut list = list;
            list[2 * k] = raw;
            let start = self.operands.len() as u32;
            self.operands.extend_from_slice(&list);
            self.code[z] = dead();
            self.code[pc] = Inst::SigReadScalar { dst: raw, sig };
            self.code[q] = Inst::Cat {
                dst,
                n,
                m,
                parts: Regs {
                    start,
                    len: parts.len,
                },
                scalars: scalars | 1 << k,
                bias: 0,
                logic: true,
                w,
            };
            self.changed = true;
            done = true;
        }
        done
    }

    /// Are the instructions at `a` and `b` in one basic block?
    fn same_block(&self, a: usize, b: usize) -> bool {
        (0..self.blocks.len()).any(|k| {
            let r = self.range(k);
            r.contains(&a) && r.contains(&b)
        })
    }
}
