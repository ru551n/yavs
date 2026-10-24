//! Instance-constant memoisation in processes (#443).
//!
//! One process function serves every instance of a design unit, so a generic, a generate
//! index or a constant is read from instance data every time: `q((i + 1) mod N)` repeats the
//! load, the addition, the `mod`, the range checks and the index arithmetic at each
//! activation, always with the same result. This pass, run last on a process, splits the
//! block of the process loop that holds such instructions at the first of them into
//!
//! - a head that tests a flag in the process frame,
//! - a slow copy of the rest of the block that is the original code (so a failing check
//!   fails with the same message at the same time) and, after the instruction that defines a
//!   value the rest of the code needs, stores it in a frame slot, and after the last
//!   instance-constant instruction sets the flag,
//! - and a fast copy without the instance-constant instructions, where those values are
//!   loaded from their slots.
//!
//! An instruction is instance-constant when it is pure, its operands are instance-constant
//! (constants, addresses, loads from instance data and the constant pool, which are frozen
//! before the simulation starts, and what is computed from them) and it can only fail in a
//! way that is the same at every execution (a check, a checked operation, a position). Moves
//! of such values, constants and addresses stay in both copies when something needs them.
//! The slow copy renames the registers it defines to fresh ones (register sharing gives them
//! cells again) so that the fast copy keeps the register counts the predecoder's fusions
//! count on.
//!
//! A descriptor the block builds (`Alloc` of a constant size, one `MakeDope` of instance-constant
//! parts, and from there on only reads on every path until the register is defined again) is
//! frame bytes too: the slow copy fills them once and the fast copy has their address (a
//! constant) and builds nothing, so the positions, lengths and layouts computed from it are
//! instance-constant like the generics it came from. The state on entry to the join of an
//! `if` on an instance-constant condition is what holds on both arms (the arm taken is the same
//! at every activation), so a statement with a null-slice test is planned as a whole.
//!
//! A reference to part of an instance signal (`SigVal`) with an instance-constant offset and
//! length is such a value too, a root of two words (loaded and stored with `Width::Sig`), and so is
//! the start of a slice (`SlicePos`) of a descriptor in frozen memory. A reference of constant
//! length or one read as a scalar is not: the interpreter fuses a single element with the read, wait
//! or edge test that uses it, and compiled code knows the length word of a constant length (it
//! drives and reads such a reference inline), which a remembered pair would lose.
//!
//! The flag and the slots are frame bytes behind the function's own frame, which start zero.
//! Debuggers keep the old shape ([`super::keep_variables`]).

// Loops index several parallel per-instruction vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Slots (and loads in the fast copy) a block may use.
const MAX_ROOTS: usize = 256;
/// Instructions the fast copy has to save, net of the flag test, to be worth a copy. Native
/// code does the saved arithmetic in a few cycles and pays for the extra code it compiles and
/// the loads of the slots, so with the LLVM tier a copy has to save more (measured on the
/// shift-register designs: a block saving 5 net loses a little compiled and wins interpreted).
const MIN_GAIN: usize = 3;
const MIN_GAIN_COMPILED: usize = 6;
/// Longest block the pass copies at the usual gain, and the longest at all. Over `COPY_BLOCK` the
/// fast copy has to save a fifth of the block: predecoding a long process costs more per
/// instruction (#683), and one that runs few activations loses by the second copy (#662).
const COPY_BLOCK: usize = 800;
const MAX_BLOCK: usize = 4096;

/// Instructions of a new block with their statements.
type Part = Vec<(Inst, crate::StmtId)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    /// Not instance-constant.
    Other,
    /// A constant, an address or a move of an instance-constant value: kept where needed.
    Free,
    /// Instance-constant, computed once.
    Memo,
    /// The `Alloc` of a descriptor whose `MakeDope` is instance-constant: the descriptor lives in
    /// frame bytes the slow copy fills once, so the address is a constant and the build goes.
    Dope,
}

/// What to do with one block.
struct Plan {
    block: usize,
    first: usize,
    kind: Vec<K>,
    /// Bytes of frame a `K::Dope` instruction owns.
    dope: Vec<u32>,
    keep: Vec<bool>,
    root: Vec<bool>,
    last_memo: usize,
    rename: Vec<u32>,
}

fn operands(f: &crate::Func, i: &Inst, mut g: impl FnMut(Reg, Role)) {
    i.visit(&mut |r, role, _| g(r, role));
    if let Some(l) = i.list()
        && let Some(rs) = f.regs(l)
    {
        for &r in rs {
            g(r, Role::Use);
        }
    }
}

pub(super) fn successors(prog: &Program, f: &crate::Func) -> Vec<Vec<usize>> {
    f.blocks
        .values()
        .iter()
        .map(|b| {
            if b.end <= b.start {
                return Vec::new();
            }
            match f.code[b.end as usize - 1] {
                Inst::Case { table, .. } => prog.cases.get(table).map_or_else(Vec::new, |t| {
                    t.arms
                        .iter()
                        .map(|a| a.2.0 as usize)
                        .chain(std::iter::once(t.default.0 as usize))
                        .collect()
                }),
                ref i => i.targets().iter().map(|t| t.0 as usize).collect(),
            }
        })
        .collect()
}

/// Is block `b` on a cycle (the process loop, or a loop inside it)?
fn on_cycle(succ: &[Vec<usize>], b: usize) -> bool {
    let mut seen = vec![false; succ.len()];
    let mut stack: Vec<usize> = succ[b].clone();
    while let Some(x) = stack.pop() {
        if x == b {
            return true;
        }
        if x < seen.len() && !seen[x] {
            seen[x] = true;
            stack.extend(&succ[x]);
        }
    }
    false
}

/// Registers live out of each block (bit sets).
fn live_out(f: &crate::Func, succ: &[Vec<usize>]) -> Vec<Vec<u64>> {
    let n = f.reg_ty.len();
    let w = n.div_ceil(64).max(1);
    let nb = f.blocks.len();
    let mut gen_ = vec![vec![0u64; w]; nb];
    let mut kill = vec![vec![0u64; w]; nb];
    for (b, blk) in f.blocks.values().iter().enumerate() {
        for pc in blk.start as usize..blk.end as usize {
            let mut uses = Vec::new();
            let mut defs = Vec::new();
            operands(f, &f.code[pc], |r, role| match role {
                Role::Use => uses.push(r.0 as usize),
                Role::Def => defs.push(r.0 as usize),
            });
            for r in uses {
                if kill[b][r / 64] & (1 << (r % 64)) == 0 {
                    gen_[b][r / 64] |= 1 << (r % 64);
                }
            }
            for r in defs {
                kill[b][r / 64] |= 1 << (r % 64);
            }
        }
    }
    let mut live_in = vec![vec![0u64; w]; nb];
    let mut out = vec![vec![0u64; w]; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut o = vec![0u64; w];
            for &s in &succ[b] {
                if s < nb {
                    for k in 0..w {
                        o[k] |= live_in[s][k];
                    }
                }
            }
            let inn: Vec<u64> = (0..w).map(|k| gen_[b][k] | (o[k] & !kill[b][k])).collect();
            if inn != live_in[b] || o != out[b] {
                live_in[b] = inn;
                out[b] = o;
                changed = true;
            }
        }
    }
    out
}

fn bit(s: &[u64], r: u32) -> bool {
    s.get(r as usize / 64)
        .is_some_and(|w| w & (1 << (r % 64)) != 0)
}

/// Classify one instruction given which registers hold instance-constant values (`inv`) and
/// which of them point into frozen memory (`ro`). Returns the kind and, for the registers
/// it defines, whether they end up instance-constant and read-only addresses.
fn classify(f: &crate::Func, i: &Inst, inv: &[bool], ro: &[bool], fused: &[bool]) -> (K, bool) {
    let ok = |r: Reg| inv.get(r.0 as usize).copied().unwrap_or(false);
    let rof = |r: Reg| ro.get(r.0 as usize).copied().unwrap_or(false);
    let plain = |r: Reg| matches!(f.reg_ty.get(r.0 as usize), Some(RegTy::I64 | RegTy::Ptr));
    match *i {
        Inst::Const { .. } | Inst::ConstF { .. } => (K::Free, false),
        Inst::AddrOf { space, .. } => (
            K::Free,
            matches!(space, AddrSpace::Inst | AddrSpace::Const(_)),
        ),
        Inst::Move { src, .. } if ok(src) => (K::Free, rof(src)),
        Inst::Load { dst, addr, w } if w != Width::Sig && rof(addr) && plain(dst) => {
            (K::Memo, false)
        }
        Inst::Bin { dst, a, b, .. } if ok(a) && ok(b) && plain(dst) => (K::Memo, false),
        Inst::BinRange {
            dst, a, b, lo, hi, ..
        } if ok(a) && ok(b) && ok(lo) && ok(hi) && plain(dst) => (K::Memo, false),
        Inst::Un { dst, a, .. } if ok(a) && plain(dst) => (K::Memo, false),
        Inst::Cmp { dst, a, b, .. } | Inst::Bool { dst, a, b, .. }
            if ok(a) && ok(b) && plain(dst) =>
        {
            (K::Memo, false)
        }
        Inst::Select { dst, c, a, b } if ok(c) && ok(a) && ok(b) && plain(dst) => (K::Memo, false),
        // The reference to a part of an instance signal: two words in a root of two slots.
        Inst::SigVal {
            dst, offset, len, ..
        } if ok(offset)
            && ok(len)
            && f.reg_ty.get(dst.0 as usize) == Some(&RegTy::Sig)
            && !fused.get(dst.0 as usize).copied().unwrap_or(false) =>
        {
            (K::Memo, false)
        }
        Inst::Pos { dst, dope, idx, .. } if rof(dope) && ok(idx) && plain(dst) => (K::Memo, false),
        Inst::DopeLen { dst, dope, .. } if rof(dope) && plain(dst) => (K::Memo, false),
        Inst::SlicePos {
            dst,
            dope,
            left,
            right,
            dir,
            ..
        } if rof(dope) && ok(left) && ok(right) && ok(dir) && plain(dst) => (K::Memo, false),
        Inst::LayoutOf { dst, desc, .. } if rof(desc) && plain(dst) => (K::Memo, true),
        Inst::Index { dst, base, pos, .. } if ok(base) && ok(pos) && plain(dst) => {
            (K::Memo, rof(base))
        }
        Inst::Check {
            v, lo, hi, kind, ..
        } if ok(v)
            && (kind == CheckKind::Null || ok(lo))
            && (!matches!(kind, CheckKind::Range | CheckKind::Index) || ok(hi)) =>
        {
            (K::Memo, false)
        }
        _ => (K::Other, false),
    }
}

/// Classify the instructions of block `b` from the state (`inv`, `ro`) on entry, leaving the
/// state on exit. `dopes` (the planned block only) holds the frame bytes of each descriptor
/// allocation that [`dope_sizes`] found: one whose `MakeDope` is instance-constant becomes a
/// [`K::Dope`], and the register points to frozen memory from there on.
fn classify_block(
    f: &crate::Func,
    b: usize,
    inv: &mut [bool],
    ro: &mut [bool],
    fused: &[bool],
    dopes: Option<&[u32]>,
) -> Vec<K> {
    let blk = f.blocks.values()[b];
    let insts = &f.code[blk.start as usize..blk.end as usize];
    let mut kind = vec![K::Other; insts.len()];
    let mut pending: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (i, ins) in insts.iter().enumerate() {
        if dopes.is_some()
            && let Inst::MakeDope { at, parts } = *ins
            && let Some(&a) = pending.get(&at.0)
            && f.regs(parts)
                .is_some_and(|rs| rs.iter().all(|r| inv[r.0 as usize]))
        {
            kind[a] = K::Dope;
            kind[i] = K::Memo;
            inv[at.0 as usize] = true;
            ro[at.0 as usize] = true;
            pending.remove(&at.0);
            continue;
        }
        let (k, is_ro) = classify(f, ins, inv, ro, fused);
        kind[i] = k;
        operands(f, ins, |r, role| {
            if role == Role::Def
                && let (Some(a), Some(c)) = (inv.get_mut(r.0 as usize), ro.get_mut(r.0 as usize))
            {
                *a = k != K::Other;
                *c = is_ro;
                pending.remove(&r.0);
            }
        });
        if let (Some(dm), Inst::Alloc { dst, .. }) = (dopes, *ins)
            && dm[i] > 0
        {
            pending.insert(dst.0, i);
        }
    }
    kind
}

/// The frame bytes (else 0) of each `Alloc` of block `b` that allocates a descriptor: a
/// constant number of bytes, one `MakeDope` of it in the block, and from there on, along every
/// path until the register is defined again, only reads of the descriptor or of the pointer.
fn dope_sizes(f: &crate::Func, succ: &[Vec<usize>], b: usize) -> Vec<u32> {
    let blk = f.blocks.values()[b];
    let insts = &f.code[blk.start as usize..blk.end as usize];
    let n = insts.len();
    let mut out = vec![0u32; n];
    let mut konst: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
    for i in 0..n {
        if let Inst::Alloc { dst, size } = insts[i]
            && let Some(&sz) = konst.get(&size.0)
            && (8..=256).contains(&sz)
            && sz % 8 == 0
            && dope_region(f, succ, b, i, dst)
        {
            out[i] = sz as u32;
        }
        match insts[i] {
            Inst::Const { dst, v } => {
                konst.insert(dst.0, v);
            }
            ref ins => operands(f, ins, |r, role| {
                if role == Role::Def {
                    konst.remove(&r.0);
                }
            }),
        }
    }
    out
}

/// How a scan of instructions ended.
enum Scan {
    /// A use that is not a read of a descriptor.
    Bad,
    /// The register is defined again.
    Defined,
    /// The end of the range.
    Open,
}

/// Scan `insts` for the uses of descriptor register `d` (`made`: its `MakeDope` has run).
fn scan_dope(f: &crate::Func, insts: &[Inst], d: Reg, made: &mut bool) -> Scan {
    for ins in insts {
        let (mut used, mut defd) = (false, false);
        operands(f, ins, |r, role| {
            if r == d {
                if role == Role::Use {
                    used = true;
                } else {
                    defd = true;
                }
            }
        });
        if used {
            let in_list = |l: Option<crate::Regs>| {
                l.and_then(|l| f.regs(l)).is_some_and(|rs| rs.contains(&d))
            };
            let ok = match *ins {
                Inst::MakeDope { at, parts } => {
                    at == d && !*made && !in_list(Some(parts)) && {
                        *made = true;
                        true
                    }
                }
                _ if !*made => false,
                Inst::DopeLen { dope, .. } => dope == d,
                Inst::Pos { dope, idx, .. } => dope == d && idx != d,
                Inst::LayoutOf { desc, .. } => desc == d,
                Inst::SubtypeCheck { .. } => true,
                Inst::Copy { dst, bytes, .. } => dst != d && bytes != d,
                Inst::Store { addr, .. } => addr != d,
                Inst::TempRoot { addr, .. } => addr != d,
                Inst::NewFrame {
                    desc, data, size, ..
                } => desc == d && data != d && size != d,
                Inst::Call { .. }
                | Inst::CallNative { .. }
                | Inst::CallProt { .. }
                | Inst::Cat { .. } => {
                    let mut fixed = false;
                    ins.visit(&mut |r, role, _| fixed |= r == d && role == Role::Use);
                    !fixed
                }
                _ => false,
            };
            if !ok {
                return Scan::Bad;
            }
        }
        if defd {
            return Scan::Defined;
        }
    }
    Scan::Open
}

/// Is the `Alloc` at `i` of block `b` (register `d`) written once, by a `MakeDope` of the same
/// block, and only read after that, on every path from it until `d` is defined again?
fn dope_region(f: &crate::Func, succ: &[Vec<usize>], b: usize, i: usize, d: Reg) -> bool {
    let blk = f.blocks.values()[b];
    let (s, e) = (blk.start as usize, blk.end as usize);
    let mut made = false;
    match scan_dope(f, &f.code[s + i + 1..e], d, &mut made) {
        Scan::Bad => return false,
        Scan::Defined => return made,
        Scan::Open => {}
    }
    if !made {
        return false;
    }
    let mut seen = vec![false; f.blocks.len()];
    let mut work: Vec<usize> = succ[b].clone();
    while let Some(x) = work.pop() {
        if x >= seen.len() || std::mem::replace(&mut seen[x], true) {
            continue;
        }
        let r = f.blocks.values()[x];
        // Coming back to the allocation's own block, the allocation ends the region.
        let end = if x == b { s + i } else { r.end as usize };
        let mut m = true;
        match scan_dope(f, &f.code[r.start as usize..end], d, &mut m) {
            Scan::Bad => return false,
            Scan::Defined => {}
            Scan::Open if x != b => work.extend(succ[x].iter().copied()),
            Scan::Open => {}
        }
    }
    true
}

/// Which registers are instance-constant, and which of them point to frozen memory.
type State = (Vec<bool>, Vec<bool>);
/// The arms and the head of an `if`: (first arm, second arm, head).
type Join = (usize, usize, usize);

/// What the planner knows about the control flow of the process.
struct Flow {
    succ: Vec<Vec<usize>>,
    /// `SigVal` results that are not remembered: of constant length, or read as a scalar.
    fused: Vec<bool>,
    /// The one predecessor of a block that nothing else enters.
    only: Vec<Option<usize>>,
    /// A block entered from the two arms of an `if` (or the head and its one arm) whose
    /// condition may be instance-constant: (first arm, second arm, head).
    join: Vec<Option<Join>>,
    exits: std::cell::RefCell<std::collections::HashMap<usize, State>>,
    sizes: std::cell::RefCell<std::collections::HashMap<usize, Vec<u32>>>,
}

impl Flow {
    fn sizes(&self, f: &crate::Func, b: usize) -> Vec<u32> {
        let mut m = self.sizes.borrow_mut();
        m.entry(b)
            .or_insert_with(|| dope_sizes(f, &self.succ, b))
            .clone()
    }

    /// The state on entry to block `b`: that on exit from its only predecessor (what the walk
    /// shared across blocks, such as the offsets of an element, is instance-constant in the
    /// blocks that follow too, #534); at the join of an `if` on an instance-constant condition,
    /// what holds on both arms (the arm taken is the same at every activation, so a value
    /// either arm defines is as constant as the one it defines on the other).
    fn entry(&self, f: &crate::Func, b: usize, depth: usize) -> State {
        let n = f.reg_ty.len();
        let none = (vec![false; n], vec![false; n]);
        if depth > 48 {
            return none;
        }
        if let Some(p) = self.only[b] {
            return self.exit(f, p, depth + 1);
        }
        let Some((a1, a2, h)) = self.join[b] else {
            return none;
        };
        let (hi, _) = self.exit(f, h, depth + 1);
        let blk = f.blocks.values()[h];
        let Inst::CondBr { c, .. } = f.code[blk.end as usize - 1] else {
            return none;
        };
        if !hi.get(c.0 as usize).copied().unwrap_or(false) {
            return none;
        }
        let (i1, r1) = self.exit(f, a1, depth + 1);
        let (i2, r2) = self.exit(f, a2, depth + 1);
        let and = |x: Vec<bool>, y: &[bool]| x.iter().zip(y).map(|(&p, &q)| p && q).collect();
        (and(i1, &i2), and(r1, &r2))
    }

    fn exit(&self, f: &crate::Func, b: usize, depth: usize) -> State {
        if let Some(x) = self.exits.borrow().get(&b) {
            return x.clone();
        }
        let (mut inv, mut ro) = self.entry(f, b, depth);
        classify_block(
            f,
            b,
            &mut inv,
            &mut ro,
            &self.fused,
            Some(&self.sizes(f, b)),
        );
        self.exits.borrow_mut().insert(b, (inv.clone(), ro.clone()));
        (inv, ro)
    }
}

/// For each block, its one predecessor when exactly one reachable block jumps to it and
/// nothing else enters it, and the arms and head when two such blocks do (see [`Flow`]).
fn only_preds(f: &crate::Func, succ: &[Vec<usize>]) -> (Vec<Option<usize>>, Vec<Option<Join>>) {
    let nb = f.blocks.len();
    let body = f.body.map(|b| b.0 as usize);
    let mut reach = vec![false; nb];
    let mut stack: Vec<usize> = std::iter::once(0)
        .chain(body)
        .chain((1..nb).filter(|&b| {
            let r = f.blocks.values()[b];
            r.end > r.start && falls_into(f, r.start as usize)
        }))
        .filter(|&b| b < nb)
        .collect();
    while let Some(b) = stack.pop() {
        if !std::mem::replace(&mut reach[b], true) {
            stack.extend(succ[b].iter().copied().filter(|&t| t < nb));
        }
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (b, s) in succ.iter().enumerate() {
        if reach[b] {
            for &t in s {
                if t < nb {
                    preds[t].push(b);
                }
            }
        }
    }
    let entered = |b: usize| {
        let r = f.blocks.values()[b];
        b == 0 || Some(b) == body || r.end <= r.start || falls_into(f, r.start as usize)
    };
    let only: Vec<Option<usize>> = (0..nb)
        .map(|b| match preds[b][..] {
            [p] if !entered(b) && p != b => Some(p),
            _ => None,
        })
        .collect();
    let join = (0..nb)
        .map(|b| {
            let [p, q] = preds[b][..] else {
                return None;
            };
            if entered(b) || p == q || p == b || q == b {
                return None;
            }
            let h = match (only[p], only[q]) {
                (Some(x), Some(y)) if x == y => x,
                (_, Some(y)) if y == p => p,
                (Some(x), _) if x == q => q,
                _ => return None,
            };
            let r = f.blocks.values()[h];
            let Inst::CondBr { t, f: e, .. } = f.code[r.end as usize - 1] else {
                return None;
            };
            let mut targets = [t.0 as usize, e.0 as usize];
            let mut want = if h == p {
                [q, b]
            } else if h == q {
                [p, b]
            } else {
                [p, q]
            };
            targets.sort_unstable();
            want.sort_unstable();
            (targets == want).then_some((p, q, h))
        })
        .collect();
    (only, join)
}

fn plan_block(f: &crate::Func, flow: &Flow, b: usize, live_out: &[u64]) -> Option<Plan> {
    let (mut inv, mut ro) = flow.entry(f, b, 0);
    let blk = f.blocks.values()[b];
    let (s, e) = (blk.start as usize, blk.end as usize);
    let n = e.checked_sub(s)?;
    if !(3..=MAX_BLOCK).contains(&n) {
        return None;
    }
    let insts = &f.code[s..e];
    let nregs = f.reg_ty.len();
    let sizes = flow.sizes(f, b);
    let kind = classify_block(f, b, &mut inv, &mut ro, &flow.fused, Some(&sizes));
    let first = kind.iter().position(|&k| matches!(k, K::Memo | K::Dope))?;
    let dope: Vec<u32> = (0..n)
        .map(|i| if kind[i] == K::Dope { sizes[i] } else { 0 })
        .collect();
    // Backward over the rest of the block: which instructions the fast copy needs.
    let mut live: Vec<bool> = (0..nregs as u32).map(|r| bit(live_out, r)).collect();
    let mut keep = vec![true; n];
    let mut root = vec![false; n];
    let mut gain = 0usize;
    for i in (first..n).rev() {
        let ins = &insts[i];
        let mut defs = Vec::new();
        let mut uses = Vec::new();
        operands(f, ins, |r, role| match role {
            Role::Def => defs.push(r.0 as usize),
            Role::Use => uses.push(r.0 as usize),
        });
        let needed = defs.iter().any(|&d| live[d]);
        match kind[i] {
            K::Other => {}
            K::Free | K::Dope => keep[i] = needed,
            K::Memo => {
                keep[i] = false;
                if needed && matches!(ins, Inst::LayoutOf { .. }) {
                    // (A layout is temporary memory: recomputed, not remembered.)
                    keep[i] = true;
                } else if needed {
                    let d = defs[0];
                    if !matches!(f.reg_ty[d], RegTy::I64 | RegTy::Sig) {
                        return None;
                    }
                    root[i] = true;
                    keep[i] = true;
                } else {
                    gain += 1;
                }
            }
        }
        for &d in &defs {
            live[d] = false;
        }
        if keep[i] && !root[i] {
            for &u in &uses {
                live[u] = true;
            }
        }
    }
    let roots = root.iter().filter(|&&r| r).count();
    if roots > MAX_ROOTS
        || (n > COPY_BLOCK && gain * 5 < n)
        || gain
            < roots / 2
                + if super::compiled() {
                    MIN_GAIN_COMPILED
                } else {
                    MIN_GAIN
                }
    {
        return None;
    }
    let last_memo = kind.iter().rposition(|&k| k == K::Memo)?;
    // Registers the slow copy can rename: defined in the rest, neither read before that
    // definition (in the slow copy, which keeps every instruction) nor read after the block.
    let mut slow: Vec<bool> = (0..nregs as u32).map(|r| bit(live_out, r)).collect();
    let mut defined = vec![false; nregs];
    for ins in insts[first..].iter().rev() {
        let mut uses = Vec::new();
        operands(f, ins, |r, role| match role {
            Role::Def => {
                defined[r.0 as usize] = true;
                slow[r.0 as usize] = false;
            }
            Role::Use => uses.push(r.0 as usize),
        });
        for u in uses {
            slow[u] = true;
        }
    }
    let rename = (0..nregs as u32)
        .filter(|&r| defined[r as usize] && !bit(live_out, r) && !slow[r as usize])
        .collect();
    Some(Plan {
        block: b,
        first,
        kind,
        dope,
        keep,
        root,
        last_memo,
        rename,
    })
}

/// Is the instruction at `pc` entered by falling through from the one before it (a resume
/// point after a `Suspend` or a call that may suspend)?
fn falls_into(f: &crate::Func, pc: usize) -> bool {
    pc > 0
        && !matches!(
            f.code[pc - 1],
            Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Ret { .. }
                | Inst::Unreachable { .. }
                | Inst::Case { .. }
        )
}

/// Does the block (a range of code) suspend, or call something that may? Merged blocks must
/// not, so that no resume point ends up inside the part the pass copies.
fn suspends(f: &crate::Func, (s, e): (usize, usize)) -> bool {
    f.code[s..e].iter().any(|i| {
        matches!(
            i,
            Inst::Suspend { .. }
                | Inst::Call { .. }
                | Inst::CallProt { .. }
                | Inst::CallIndirect { .. }
                | Inst::CallForeign { .. }
        )
    })
}

/// Join each block that only the unconditional jump ending another block enters to that block
/// (the straight-line pieces lowering leaves between the arms of an `if`), so that the planner
/// sees the whole run. The emptied block stays, unreachable. Returns whether anything moved.
fn merge_chains(prog: &Program, f: &mut crate::Func) -> bool {
    let succ = successors(prog, f);
    let nb = f.blocks.len();
    let body = f.body.map(|b| b.0 as usize);
    // Blocks reachable from the entry (and the body): a dead block does not count as a
    // predecessor.
    let mut reach = vec![false; nb];
    // Blocks a resume point enters (after a suspension, by falling through) are roots too.
    let mut stack: Vec<usize> = std::iter::once(0)
        .chain(body)
        .chain((1..nb).filter(|&b| {
            let r = f.blocks.values()[b];
            r.end > r.start && falls_into(f, r.start as usize)
        }))
        .filter(|&b| b < nb)
        .collect();
    while let Some(b) = stack.pop() {
        if !std::mem::replace(&mut reach[b], true) {
            stack.extend(succ[b].iter().copied().filter(|&t| t < nb));
        }
    }
    let mut preds = vec![0u32; nb];
    for (b, s) in succ.iter().enumerate() {
        if reach[b] {
            for &t in s {
                if t < nb {
                    preds[t] += 1;
                }
            }
        }
    }
    let ranges: Vec<(usize, usize)> = f
        .blocks
        .values()
        .iter()
        .map(|b| (b.start as usize, b.end as usize))
        .collect();
    let mut chain: Vec<Vec<usize>> = (0..nb).map(|b| vec![b]).collect();
    let mut merged = false;
    for a in 0..nb {
        if chain[a].is_empty() || !reach[a] {
            continue;
        }
        loop {
            let last = *chain[a].last().unwrap_or(&a);
            let (s, e) = ranges[last];
            if e <= s {
                break;
            }
            let Inst::Br { to } = f.code[e - 1] else {
                break;
            };
            let b = to.0 as usize;
            if b >= nb
                || b == a
                || b == 0
                || Some(b) == body
                || preds[b] != 1
                || chain[b].is_empty()
                || chain[b][0] != b
                || ranges[b].1 <= ranges[b].0
                || falls_into(f, ranges[b].0)
                || suspends(f, ranges[b])
                || chain[a].iter().any(|&c| suspends(f, ranges[c]))
            {
                break;
            }
            let moved = std::mem::take(&mut chain[b]);
            chain[a].extend(moved);
            merged = true;
        }
    }
    if !merged {
        return false;
    }
    let mut code = Vec::with_capacity(f.code.len());
    let mut loc = Vec::with_capacity(f.code.len());
    let mut new_ranges = vec![(0u32, 0u32); nb];
    for b in 0..nb {
        let start = code.len() as u32;
        let parts = &chain[b];
        for (k, &c) in parts.iter().enumerate() {
            let (s, mut e) = ranges[c];
            // The jump to the next piece goes away.
            if k + 1 < parts.len() {
                e -= 1;
            }
            code.extend_from_slice(&f.code[s..e]);
            loc.extend_from_slice(&f.loc[s..e]);
        }
        if code.len() as u32 == start {
            // A block merged into another stays (blocks are numbered), unreachable, as one jump.
            let (s, e) = ranges[b];
            let l = if e > s { f.loc[e - 1] } else { f.loc[0] };
            code.push(Inst::Br {
                to: crate::BlockId(b as u32),
            });
            loc.push(l);
        }
        new_ranges[b] = (start, code.len() as u32);
    }
    for (i, &(s, e)) in new_ranges.iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(crate::BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
    true
}

/// Apply the pass to process `f`.
pub(super) fn run(prog: &Program, f: &mut crate::Func) {
    if f.kind != FuncKind::Process || f.code.len() > 6000 {
        return;
    }
    let (code0, loc0, blocks0) = (
        f.code.clone(),
        f.loc.clone(),
        f.blocks
            .values()
            .iter()
            .map(|b| (b.start, b.end))
            .collect::<Vec<_>>(),
    );
    merge_chains(prog, f);
    let restore = |f: &mut crate::Func| {
        f.code = code0.clone();
        f.loc = loc0.clone();
        for (i, &(s, e)) in blocks0.iter().enumerate() {
            if let Some(b) = f.blocks.get_mut(crate::BlockId(i as u32)) {
                b.start = s;
                b.end = e;
            }
        }
    };
    let succ = successors(prog, f);
    let nb = f.blocks.len();
    let (only, join) = only_preds(f, &succ);
    // A reference of constant length or read as a scalar stays a `SigVal` (see the module
    // documentation).
    let mut consts = vec![0u8; f.reg_ty.len()];
    let mut fused = vec![false; f.reg_ty.len()];
    for i in &f.code {
        match *i {
            Inst::Const { dst, .. } => {
                consts[dst.0 as usize] = consts[dst.0 as usize].saturating_add(1)
            }
            Inst::SigReadScalar { sig, .. } | Inst::Edge { sig, .. } => {
                fused[sig.0 as usize] = true
            }
            _ => {}
        }
    }
    let mut defs = vec![0u8; f.reg_ty.len()];
    for i in &f.code {
        operands(f, i, |r, role| {
            if role == Role::Def {
                defs[r.0 as usize] = defs[r.0 as usize].saturating_add(1);
            }
        });
    }
    for i in &f.code {
        if let Inst::SigVal { dst, len, .. } = *i
            && consts[len.0 as usize] == 1
            && defs[len.0 as usize] == 1
        {
            fused[dst.0 as usize] = true;
        }
    }
    let flow = Flow {
        fused,
        succ,
        only,
        join,
        exits: Default::default(),
        sizes: Default::default(),
    };
    let (succ, only) = (&flow.succ, &flow.only);
    // Cheap test first: some cyclic block with a load from frozen memory, or one that follows
    // another block alone (the values it needs may be instance-constant there).
    let candidate = |b: usize| {
        let blk = f.blocks.values()[b];
        (only[b].is_some()
            || flow.join[b].is_some()
            || f.code[blk.start as usize..blk.end as usize]
                .iter()
                .any(|i| {
                    matches!(
                        i,
                        Inst::AddrOf {
                            space: AddrSpace::Inst | AddrSpace::Const(_),
                            ..
                        }
                    )
                }))
            && on_cycle(succ, b)
    };
    let cands: Vec<usize> = (0..nb).filter(|&b| candidate(b)).collect();
    if cands.is_empty() {
        restore(f);
        return;
    }
    let out = live_out(f, succ);
    let plans: Vec<Plan> = cands
        .into_iter()
        .filter_map(|b| plan_block(f, &flow, b, &out[b]))
        .collect();
    if plans.is_empty() {
        restore(f);
        return;
    }
    let base = f.frame.next_multiple_of(8);
    let mut slots = 0u32;
    let mut code: Vec<Inst> = Vec::with_capacity(f.code.len() + 64);
    let mut loc = Vec::with_capacity(code.capacity());
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut extra: Vec<(u32, u32)> = Vec::new();
    let fresh = |f: &mut crate::Func, t: RegTy| -> Reg {
        let r = Reg(f.reg_ty.len() as u32);
        f.reg_ty.push(t);
        if t == RegTy::Sig {
            f.reg_ty.push(RegTy::I64);
        }
        r
    };
    let mut by_block: Vec<Option<&Plan>> = vec![None; nb];
    for p in &plans {
        by_block[p.block] = Some(p);
    }
    // New blocks are numbered from `nb`; every plan adds two (fast, slow).
    let mut next_block = nb as u32;
    let mut new_blocks: Vec<(Part, Part)> = Vec::new();
    let old_blocks: Vec<(u32, u32)> = f.blocks.values().iter().map(|b| (b.start, b.end)).collect();
    for (b, &(s, e)) in old_blocks.iter().enumerate() {
        let start = code.len() as u32;
        let Some(p) = by_block[b] else {
            code.extend_from_slice(&f.code[s as usize..e as usize]);
            loc.extend_from_slice(&f.loc[s as usize..e as usize]);
            ranges.push((start, code.len() as u32));
            continue;
        };
        let (s, e) = (s as usize, e as usize);
        let first_pc = s + p.first;
        code.extend_from_slice(&f.code[s..first_pc]);
        loc.extend_from_slice(&f.loc[s..first_pc]);
        let st = f.loc[first_pc];
        let (fast_id, slow_id) = (next_block, next_block + 1);
        next_block += 2;
        let flag_off = base + 8 * slots;
        slots += 1;
        let rf = fresh(f, RegTy::Ptr);
        let rv = fresh(f, RegTy::I64);
        for i in [
            Inst::AddrOf {
                dst: rf,
                space: AddrSpace::Frame,
                off: flag_off,
            },
            Inst::Load {
                dst: rv,
                addr: rf,
                w: Width::B8,
            },
            Inst::CondBr {
                c: rv,
                t: crate::BlockId(fast_id),
                f: crate::BlockId(slow_id),
            },
        ] {
            code.push(i);
            loc.push(st);
        }
        ranges.push((start, code.len() as u32));
        // Slots of the roots, in order.
        let mut root_off = vec![0u32; e - s];
        // A root is one word, or two for a signal reference.
        let widths: Vec<Width> = (0..e - s)
            .map(|i| {
                let mut w = Width::B8;
                f.code[s + i].visit(&mut |r, role, _| {
                    if role == Role::Def && f.reg_ty[r.0 as usize] == RegTy::Sig {
                        w = Width::Sig;
                    }
                });
                w
            })
            .collect();
        for i in p.first..e - s {
            if p.root[i] {
                root_off[i] = base + 8 * slots;
                slots += if widths[i] == Width::Sig { 2 } else { 1 };
            }
        }
        // Frame bytes of the descriptors.
        let mut dope_off = vec![0u32; e - s];
        for i in p.first..e - s {
            if p.kind[i] == K::Dope {
                dope_off[i] = base + 8 * slots;
                slots += p.dope[i] / 8;
            }
        }
        // Fast copy.
        let mut fast = Vec::new();
        for i in p.first..e - s {
            let ins = f.code[s + i];
            let l = f.loc[s + i];
            if p.root[i] {
                let mut d = Reg(0);
                ins.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        d = r;
                    }
                });
                let ra = fresh(f, RegTy::Ptr);
                fast.push((
                    Inst::AddrOf {
                        dst: ra,
                        space: AddrSpace::Frame,
                        off: root_off[i],
                    },
                    l,
                ));
                fast.push((
                    Inst::Load {
                        dst: d,
                        addr: ra,
                        w: widths[i],
                    },
                    l,
                ));
            } else if p.kind[i] == K::Dope {
                if p.keep[i]
                    && let Inst::Alloc { dst, .. } = ins
                {
                    fast.push((
                        Inst::AddrOf {
                            dst,
                            space: AddrSpace::Frame,
                            off: dope_off[i],
                        },
                        l,
                    ));
                }
            } else if p.keep[i] || p.kind[i] == K::Other {
                fast.push((ins, l));
            }
        }
        // Slow copy: the original, renamed, with the stores.
        let mut map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for &r in &p.rename {
            let t = f.reg_ty[r as usize];
            let nr = fresh(f, t);
            map.insert(r, nr.0);
        }
        let scratch = fresh(f, RegTy::Ptr);
        let one = fresh(f, RegTy::I64);
        let mut slow = Vec::new();
        for i in p.first..e - s {
            let mut ins = f.code[s + i];
            let l = f.loc[s + i];
            ins.visit_mut(&mut |r, _, _| {
                if let Some(&m) = map.get(&r.0) {
                    r.0 = m;
                }
            });
            if let Some(list) = ins.list()
                && let Some(rs) = f.regs(list).map(<[Reg]>::to_vec)
                && rs.iter().any(|r| map.contains_key(&r.0))
            {
                let start = f.operands.len() as u32;
                f.operands.extend(
                    rs.iter()
                        .map(|r| Reg(map.get(&r.0).copied().unwrap_or(r.0))),
                );
                if let Some(slot) = ins.list_mut() {
                    *slot = crate::Regs {
                        start,
                        len: rs.len() as u32,
                    };
                }
            }
            if p.kind[i] == K::Dope
                && let Inst::Alloc { dst, .. } = ins
            {
                ins = Inst::AddrOf {
                    dst,
                    space: AddrSpace::Frame,
                    off: dope_off[i],
                };
            }
            slow.push((ins, l));
            if p.root[i] {
                let mut d = Reg(0);
                ins.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        d = r;
                    }
                });
                slow.push((
                    Inst::AddrOf {
                        dst: scratch,
                        space: AddrSpace::Frame,
                        off: root_off[i],
                    },
                    l,
                ));
                slow.push((
                    Inst::Store {
                        addr: scratch,
                        src: d,
                        w: widths[i],
                    },
                    l,
                ));
            }
            if i == p.last_memo {
                slow.push((Inst::Const { dst: one, v: 1 }, l));
                slow.push((
                    Inst::AddrOf {
                        dst: scratch,
                        space: AddrSpace::Frame,
                        off: flag_off,
                    },
                    l,
                ));
                slow.push((
                    Inst::Store {
                        addr: scratch,
                        src: one,
                        w: Width::B8,
                    },
                    l,
                ));
            }
        }
        new_blocks.push((fast, slow));
    }
    for (fast, slow) in new_blocks {
        for part in [fast, slow] {
            let start = code.len() as u32;
            for (i, l) in part {
                code.push(i);
                loc.push(l);
            }
            extra.push((start, code.len() as u32));
        }
    }
    for (i, &(s, e)) in ranges.iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(crate::BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    for (s, e) in extra {
        f.blocks.push(crate::Block { start: s, end: e });
    }
    f.code = code;
    f.loc = loc;
    f.frame = base + 8 * slots;
}
