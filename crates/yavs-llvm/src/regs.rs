//! Which registers of a frame-mode function a leaving path needs in the frame (#432).
//!
//! Compiled code runs on the interpreter's register file, so every definition used to be a store.
//! Most of them are temporaries nobody reads from the frame: the interpreter reads a register
//! only where compiled code leaves it (a guard failing, a stepped instruction, a block edge that
//! the interpreter may enter). `Live` gives the registers live before each instruction, and
//! `Gen::flush` stores a definition only where it is live and the frame is looked at. `preloaded`
//! names the registers the interpreter starts every frame with (defined once by a constant or an
//! address): compiled code never stores those.

use yavs_ir::{AddrSpace, Func, Inst, Program, Reg, RegTy, Role};

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
    pub fn new(prog: &Program, f: &Func) -> Option<Live> {
        let n = f.code.len();
        let words = f.reg_ty.len().div_ceil(64).max(1);
        if n.checked_mul(words)? > MAX_WORDS {
            return None;
        }
        // Uses and definitions of each instruction, and its successors.
        let mut uses = vec![0u64; n * words];
        let mut defs = vec![0u64; n * words];
        let mut succ: Vec<Vec<u32>> = Vec::with_capacity(n);
        let start = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start);
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
            let next = (pc + 1 < n).then_some(pc as u32 + 1);
            let s: Vec<u32> = match *i {
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
                Inst::Ret { .. } | Inst::Unreachable { .. } => Vec::new(),
                _ => next.into_iter().collect(),
            };
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
