//! Dead definitions of registers that are defined more than once (#444).
//!
//! `Opt::dce` removes a pure instruction when nobody reads its register at all. Registers the
//! lowering merges by assignment on several paths (the flag of `a and then b`, the result of
//! a select, a loop variable) are read somewhere, so their definitions stay, even on paths
//! that no longer reach a reader after jump threading and folding. A backward liveness
//! analysis over the blocks finds the definitions whose value no path reads before the
//! register is written again, and removes the pure ones.
//!
//! Only registers defined on several paths take part: one defined once may be read where its
//! definition has not run (the interpreter starts a frame with the value of a register
//! defined once by a constant or an address, and merged constants share one).

use super::*;

type Bits = Vec<u64>;

impl Opt<'_> {
    /// Remove pure definitions no path reads; whether any went.
    pub(super) fn dead_defs(&mut self) -> bool {
        let n = self.defs.len();
        let nb = self.blocks.len();
        if !self.nosusp || self.f.body.is_some() || self.f.flags.contains(FuncFlags::MAY_SUSPEND) {
            return false;
        }
        let mut any = false;
        // Removing a definition can make the ones before it dead: go again once.
        for _ in 0..2 {
            let mut ndefs = vec![0u8; n];
            for i in &self.code {
                if !is_dead(i) {
                    for_defs(i, |r| {
                        if let Some(d) = ndefs.get_mut(r.0 as usize) {
                            *d = d.saturating_add(1);
                        }
                    });
                }
            }
            // The registers that take part, numbered densely.
            let mut idx = vec![u32::MAX; n];
            let mut m = 0usize;
            for (r, &d) in ndefs.iter().enumerate() {
                if d > 1 {
                    idx[r] = m as u32;
                    m += 1;
                }
            }
            // Something to remove: a pure definition of one of them.
            if !self.code.iter().any(|i| {
                !is_dead(i) && removable_dst(i).is_some_and(|d| idx[d.0 as usize] != u32::MAX)
            }) {
                break;
            }
            let w = m.div_ceil(64).max(1);
            let reach = self.reachable();
            // upward-exposed uses and definitions per block
            let mut uses: Vec<Bits> = vec![vec![0; w]; nb];
            let mut defs: Vec<Bits> = vec![vec![0; w]; nb];
            for b in 0..nb {
                if !reach[b] {
                    continue;
                }
                for pc in self.range(b) {
                    let i = &self.code[pc];
                    if is_dead(i) {
                        continue;
                    }
                    self.uses_of(i, |r| {
                        let k = idx[r.0 as usize];
                        if k != u32::MAX {
                            let k = k as usize;
                            if defs[b][k / 64] >> (k % 64) & 1 == 0 {
                                uses[b][k / 64] |= 1 << (k % 64);
                            }
                        }
                    });
                    for_defs(i, |r| {
                        let k = idx[r.0 as usize];
                        if k != u32::MAX {
                            let k = k as usize;
                            defs[b][k / 64] |= 1 << (k % 64);
                        }
                    });
                }
            }
            let mut live_in: Vec<Bits> = uses.clone();
            let mut live_out: Vec<Bits> = vec![vec![0; w]; nb];
            let mut again = true;
            while again {
                again = false;
                for b in (0..nb).rev() {
                    if !reach[b] {
                        continue;
                    }
                    let mut out = vec![0u64; w];
                    let mut k = 0;
                    while let Some(s) = self.succ(b, k) {
                        k += 1;
                        for (o, i) in out.iter_mut().zip(&live_in[s]) {
                            *o |= *i;
                        }
                    }
                    let inn: Bits = (0..w)
                        .map(|x| uses[b][x] | (out[x] & !defs[b][x]))
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
                if !reach[b] {
                    continue;
                }
                let mut live = live_out[b].clone();
                for pc in self.range(b).rev() {
                    let i = self.code[pc];
                    if is_dead(&i) {
                        continue;
                    }
                    if let Some(d) = removable_dst(&i) {
                        let k = idx[d.0 as usize];
                        if k != u32::MAX && live[k as usize / 64] >> (k % 64) & 1 == 0 {
                            self.code[pc] = dead();
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
                    self.uses_of(&i, |r| {
                        let k = idx[r.0 as usize];
                        if k != u32::MAX {
                            live[k as usize / 64] |= 1 << (k % 64);
                        }
                    });
                }
            }
            if !removed {
                break;
            }
            any = true;
        }
        if any {
            self.changed = true;
        }
        any
    }

    /// The registers the instruction reads, operand lists included.
    fn uses_of(&self, i: &Inst, mut f: impl FnMut(Reg)) {
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
}
