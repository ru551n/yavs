//! Per-function facts computed once, before a function first runs (#412).
//!
//! A register defined exactly once by a `Const`, `ConstF` or `AddrOf` holds the same value
//! whenever it is read, so a new frame starts with it instead of every pass over the
//! instruction executing it again (lowering emits these at each use, in loops too). The
//! instruction stays in the code, so compiled code, pc numbering and the debug loop are
//! unchanged; the dispatch loop skips it through `next`. An unconditional jump is skipped
//! the same way: `next` leads to its target.

use yavs_ir::{AddrSpace, Func, Inst, Role};

use crate::exec::{CONST, FRAME, INST, PKG, R, SHARED, mk_addr};

pub(crate) struct Pre {
    /// Initial registers: the preloaded values, zero elsewhere.
    init: Vec<u64>,
    /// Registers holding an address in the frame itself: (register, offset).
    frame: Vec<(u32, u64)>,
    /// For each pc (and one past the end), the first pc at or after it that is not a
    /// preloaded definition.
    pub next: Vec<u32>,
}

impl Pre {
    /// Set `regs` up for a frame at index `idx`: the preloaded registers have their values,
    /// the others are zero.
    ///
    /// ponytail: zeroing is not needed for correctness (#430: the verifier proves every other
    /// register is written before it is read; a sparse fill must list preloaded zeros too),
    /// but the collection scans registers conservatively and stale ones kept garbage alive
    /// (+25% run time, +31% peak memory on a long testbench). Leave them stale once the scan
    /// is precise (#422).
    pub(crate) fn fill(&self, regs: &mut Vec<u64>, idx: u32) -> R<()> {
        regs.clone_from(&self.init);
        for &(r, off) in &self.frame {
            regs[r as usize] = mk_addr(FRAME, idx, off)?;
        }
        Ok(())
    }

    pub(crate) fn new(f: &Func) -> Pre {
        let n = f.reg_ty.len();
        let mut defs = vec![0u8; n];
        for inst in &f.code {
            inst.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
        let mut init = vec![0u64; n];
        let mut frame = Vec::new();
        let mut next = vec![f.code.len() as u32; f.code.len() + 1];
        for pc in (0..f.code.len()).rev() {
            let once = |r: yavs_ir::Reg| defs.get(r.0 as usize) == Some(&1);
            let hit = match f.code[pc] {
                Inst::Const { dst, v } if once(dst) => {
                    init[dst.0 as usize] = v as u64;
                    true
                }
                Inst::ConstF { dst, v } if once(dst) => {
                    init[dst.0 as usize] = v.to_bits();
                    true
                }
                Inst::AddrOf { dst, space, off } if once(dst) => {
                    let off = u64::from(off);
                    let a = match space {
                        AddrSpace::Frame => {
                            frame.push((dst.0, off));
                            Some(0)
                        }
                        AddrSpace::Up(_) => None,
                        AddrSpace::Inst => mk_addr(INST, 0, off).ok(),
                        AddrSpace::Pkg(p) => mk_addr(PKG, p, off).ok(),
                        AddrSpace::Const(c) => mk_addr(CONST, c.0, off).ok(),
                        AddrSpace::Shared => mk_addr(SHARED, 0, off).ok(),
                    };
                    if let Some(a) = a {
                        init[dst.0 as usize] = a;
                    }
                    a.is_some()
                }
                _ => false,
            };
            next[pc] = if hit { next[pc + 1] } else { pc as u32 };
        }
        // An unconditional jump costs nothing: execution goes on at its target (through
        // other jumps and definitions the frame started with). Not to the block a machine
        // pauses at.
        let jump = |pc: usize| match f.code.get(pc) {
            Some(&Inst::Br { to }) if f.body != Some(to) => f.blocks.get(to).map(|b| b.start),
            _ => None,
        };
        let thread: Vec<Option<u32>> = (0..f.code.len())
            .map(|pc| {
                let (mut at, mut hops) = (pc, 0);
                while let Some(start) = jump(at) {
                    at = next[start as usize] as usize;
                    hops += 1;
                    if hops > 32 || at == pc {
                        return None;
                    }
                }
                (hops > 0).then_some(at as u32)
            })
            .collect();
        for n in &mut next {
            if let Some(Some(t)) = thread.get(*n as usize) {
                *n = *t;
            }
        }
        Pre { init, frame, next }
    }
}
