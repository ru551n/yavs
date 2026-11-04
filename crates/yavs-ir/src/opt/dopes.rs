//! Descriptors read in registers: where a `MakeDope` of run-time bounds fills a temporary or a
//! local's slot, or the bounds are in a parameter's slots (`Param::flat`), and the reader can see
//! the registers, `DopeLen` is `BoundsLen`, `Pos` `BoundsPos`, `SlicePos` `BoundsSlice`, a
//! one-dimensional `SubtypeCheck` a length `Check`, the size of a layout arithmetic on the length
//! and a `Copy` of the descriptor another `MakeDope`. `dead_defs` and the walk's `dce` drop what
//! nothing reads any more. Works on the code before registers are shared, where nearly every
//! register is defined once. `YAVS_NO_DOPES=1` turns it off (fingerprint `dopes`).

use std::sync::atomic::{AtomicU64, Ordering};

use super::cfg::counts;
use super::*;

/// Instructions the pass looked at to find what writes or lets escape the blocks of descriptors.
static SCANS: AtomicU64 = AtomicU64::new(0);

/// How many instructions the pass looked at in this process: a function's code once or twice per
/// round, never once per candidate (a work counter: `YAVS_COUNTERS`).
pub fn scans() -> u64 {
    SCANS.load(Ordering::Relaxed)
}

/// Is the pass on?
pub(super) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_DOPES")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// The bounds of a descriptor a block allocates and fills with registers defined once.
#[derive(Clone, Copy)]
struct Made {
    block: usize,
    /// The instruction after the `MakeDope`: readers before it in the block are not readers of
    /// what it wrote.
    from: usize,
    bounds: (Reg, Reg, Reg),
}

/// Calls `out` with each register whose block `i` may write or let escape: those `i` does not
/// only read. (One pass over the code with this finds, for all registers at once, the ones
/// something besides a read names.)
fn not_read_only(f: &crate::Func, i: &Inst, mut out: impl FnMut(Reg)) {
    let list = |l: crate::Regs, out: &mut dyn FnMut(Reg)| {
        f.regs(l).unwrap_or(&[]).iter().for_each(|&r| out(r))
    };
    match *i {
        Inst::Alloc { dst, size } => {
            if size != dst {
                out(size)
            }
        }
        Inst::MakeDope { parts, .. } => list(parts, &mut out),
        Inst::DopeLen { .. } | Inst::SubtypeCheck { .. } | Inst::LayoutOf { .. } => {}
        Inst::Pos { dst, idx, .. } => {
            out(dst);
            out(idx);
        }
        Inst::SlicePos {
            dst,
            left,
            right,
            dir,
            ..
        } => [dst, left, right, dir].into_iter().for_each(out),
        Inst::Load { dst, .. } | Inst::Copy { dst, .. } => out(dst),
        Inst::NewFrame {
            dst, data, size, ..
        }
        | Inst::NewFill {
            dst, data, size, ..
        } => [dst, data, size].into_iter().for_each(out),
        Inst::Store { addr, .. } => out(addr),
        Inst::Call { .. }
        | Inst::CallProt { .. }
        | Inst::CallIndirect { .. }
        | Inst::Ret { .. } => {}
        _ => {
            i.visit(&mut |x, role, _| {
                if role == Role::Use {
                    out(x)
                }
            });
            if let Some(l) = i.list() {
                list(l, &mut out);
            }
        }
    }
}

/// Drops from `keep` every register some instruction of `f` does not only read.
fn keep_read_only(f: &crate::Func, keep: &mut FxMap<Reg, impl Sized>) {
    let mut scans = 0;
    for i in &f.code {
        if keep.is_empty() {
            break;
        }
        scans += 1;
        not_read_only(f, i, |r| {
            keep.remove(&r);
        });
    }
    SCANS.fetch_add(scans, Ordering::Relaxed);
}

/// `dom[b]`: the blocks that dominate `b`, as bits; `None` for a function too big for the sets.
fn dominators(prog: &Program, f: &crate::Func) -> Option<Vec<Vec<u64>>> {
    let nb = f.blocks.len();
    let w = nb.div_ceil(64).max(1);
    if nb.checked_mul(w)? > 1 << 20 {
        return None;
    }
    let succ = super::memo::successors(prog, f);
    let mut pred = vec![Vec::new(); nb];
    for (b, ss) in succ.iter().enumerate() {
        for &s in ss {
            if s >= nb {
                return None;
            }
            pred[s].push(b);
        }
    }
    let mut dom = vec![vec![!0u64; w]; nb];
    if let Some(d) = dom.first_mut() {
        d.fill(0);
        d[0] = 1;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..nb {
            let mut x = vec![!0u64; w];
            for &p in &pred[b] {
                x.iter_mut().zip(&dom[p]).for_each(|(a, c)| *a &= c);
            }
            x[b / 64] |= 1 << (b % 64);
            if x != dom[b] {
                dom[b] = x;
                changed = true;
            }
        }
    }
    Some(dom)
}

/// Can no register live across a suspension of `f`, and no other frame reach its slots?
fn closed(f: &crate::Func) -> bool {
    !f.nested
        && !f.flags.contains(FuncFlags::MAY_SUSPEND)
        && !f.code.iter().any(|i| matches!(i, Inst::Suspend { .. }))
}

/// What `frame_home` asks of the code, found in one pass: whether nothing can reach the frame's
/// slots from outside, the first `AddrOf` of each register and every `AddrOf` by offset.
struct Homes {
    closed: bool,
    first: FxMap<Reg, u32>,
    by_off: Vec<(u32, Reg)>,
}

impl Homes {
    fn new(f: &crate::Func) -> Self {
        let mut first = FxMap::default();
        let mut by_off = Vec::new();
        for i in &f.code {
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } = *i
            {
                first.entry(dst).or_insert(off);
                by_off.push((off, dst));
            }
        }
        by_off.sort_unstable();
        Homes {
            closed: closed(f),
            first,
            by_off,
        }
    }

    /// Is `at` the one name of the 24 bytes of the frame's slot it addresses? (A local array's own
    /// descriptor, made once in its slot.)
    fn frame_home(&self, at: Reg) -> bool {
        let Some(&off) = self.first.get(&at) else {
            return false;
        };
        // Every other address `o` of the frame has `o + 8 <= off || o >= off + 24`.
        let (lo, hi) = (off.saturating_sub(7), off.saturating_add(24));
        let from = self.by_off.partition_point(|&(o, _)| o < lo);
        self.closed
            && self.by_off[from..]
                .iter()
                .take_while(|&&(o, _)| o < hi)
                .all(|&(_, d)| d == at)
    }
}

/// The descriptors of `f` whose bounds a reader can take from registers.
fn made(f: &crate::Func, defs: &[u8], frame: bool) -> FxMap<Reg, Made> {
    let once = |r: Reg| defs.get(r.0 as usize) == Some(&1);
    let mut out: FxMap<Reg, Made> = FxMap::default();
    let mut bad: Vec<Reg> = Vec::new();
    let mut homes: Option<Homes> = None;
    // The registers an `Alloc` of this block filled and nothing since has written or let escape.
    let mut fresh: FxMap<Reg, ()> = FxMap::default();
    let mut scans = 0;
    for (b, blk) in f.blocks.values().iter().enumerate() {
        let (s, e) = (blk.start as usize, blk.end as usize);
        fresh.clear();
        for pc in s..e {
            let i = &f.code[pc];
            if let Inst::MakeDope { at, parts } = *i
                && let Some(&[l, r, d]) = f.regs(parts)
            {
                let alloc = fresh.contains_key(&at);
                if (alloc || (frame && homes.get_or_insert_with(|| Homes::new(f)).frame_home(at)))
                    && once(at)
                    && once(l)
                    && once(r)
                    && once(d)
                    && at != l
                    && at != r
                    && at != d
                    && out
                        .insert(
                            at,
                            Made {
                                block: b,
                                from: pc + 1,
                                bounds: (l, r, d),
                            },
                        )
                        .is_some()
                {
                    bad.push(at);
                }
            }
            scans += 1;
            not_read_only(f, i, |r| {
                fresh.remove(&r);
            });
            if let Inst::Alloc { dst, .. } = *i {
                fresh.insert(dst, ());
            }
        }
    }
    SCANS.fetch_add(scans, Ordering::Relaxed);
    for r in bad {
        out.remove(&r);
    }
    // Anything that may write the block: leave it alone. (A reader before the `MakeDope` of the
    // block is no reader of it, and a register defined once has no earlier value.)
    keep_read_only(f, &mut out);
    out
}

/// Size of an array of `n` elements when its layout depends on the length alone: the size is
/// `(n + 63) / 64 * 32` for `Some(true)`, `n * stride` for `Some(false)`.
fn size_form(prog: &Program, ty: TyId) -> Option<(bool, u64)> {
    let types = &prog.types;
    if Ty::desc_size(types, ty) != Some(24)
        || !matches!(types.get(ty), Some(Ty::Array { dims: 1, .. }))
    {
        return None;
    }
    let of = |n: i64| {
        let mut d = [0u8; 24];
        d[8..16].copy_from_slice(&(n - 1).to_le_bytes());
        Ty::layout(types, &prog.consts, ty, &d).ok().map(|x| x.0)
    };
    let (a, b) = (of(1)?, of(65)?);
    if !a.offsets.is_empty() {
        return None;
    }
    if Ty::logic_leaf(types, ty).is_some() {
        // One element per position (an array of arrays of logic counts its rows' elements).
        (a.count == 1 && a.size == 32 && b.size == 64).then_some((true, 32))
    } else {
        (a.stride > 0 && a.size == a.stride && b.size == 65 * a.stride).then_some((false, a.stride))
    }
}

/// The slots of the descriptors in the parameters of a function that cannot suspend (a register
/// lives across no suspension): the address registers that name them, with the slot.
fn flat_slots(f: &crate::Func, defs: &[u8]) -> FxMap<Reg, u32> {
    let mut out = FxMap::default();
    if !closed(f) || !f.params.iter().any(|p| p.flat) {
        return out;
    }
    for i in &f.code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
            && defs.get(dst.0 as usize) == Some(&1)
            && f.params.iter().any(|p| p.flat && p.slot + 8 == off)
        {
            out.insert(dst, off);
        }
    }
    keep_read_only(f, &mut out);
    out
}

/// Rewrite the reads of descriptors the code made itself or got in its parameters' slots. Returns
/// whether anything changed. A function with a loop keeps `Pos` and `SlicePos` of a descriptor in
/// the frame and of a block: the element loops and fused runs of the tiers read those (the frame's
/// words are one load each in compiled code).
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    let regs = !super::cfg::has_back_edge(f);
    run_with(prog, f, regs)
}

/// The same with every read in registers, for a body about to be inlined: its bounds become the
/// caller's registers.
pub(super) fn run_regs(prog: &Program, f: &mut crate::Func) -> bool {
    run_with(prog, f, true)
}

fn run_with(prog: &Program, f: &mut crate::Func, regs: bool) -> bool {
    let has_dope = f.code.iter().any(|i| matches!(i, Inst::MakeDope { .. }));
    if !has_dope && !f.params.iter().any(|p| p.flat) {
        return false;
    }
    let mut any = false;
    // A rewritten `Copy` is a descriptor of its own: its readers come with the next round.
    for k in 0..3 {
        if !round(prog, f, regs, regs && k == 0) {
            break;
        }
        any = true;
    }
    any
}

fn round(prog: &Program, f: &mut crate::Func, regs: bool, params: bool) -> bool {
    let (defs, uses) = counts(f);
    let mut known = made(f, &defs, regs);
    let slots = if params {
        flat_slots(f, &defs)
    } else {
        FxMap::default()
    };
    if known.is_empty() && slots.is_empty() {
        return false;
    }
    let Some(dom) = dominators(prog, f) else {
        return false;
    };
    // Each parameter's words are read once, at the start of the entry block.
    let mut entry: Vec<Inst> = Vec::new();
    let mut read: FxMap<u32, (Reg, Reg, Reg)> = FxMap::default();
    for (&r, &off) in &slots {
        let bounds = *read.entry(off).or_insert_with(|| {
            let mut word = |k: u32| {
                let a = Reg(f.reg_ty.len() as u32);
                f.reg_ty.push(RegTy::Ptr);
                let dst = Reg(f.reg_ty.len() as u32);
                f.reg_ty.push(RegTy::I64);
                entry.push(Inst::AddrOf {
                    dst: a,
                    space: AddrSpace::Frame,
                    off: off + 8 * k,
                });
                entry.push(Inst::Load {
                    dst,
                    addr: a,
                    w: Width::B8,
                });
                dst
            };
            (word(0), word(1), word(2))
        });
        known.insert(
            r,
            Made {
                block: 0,
                from: 0,
                bounds,
            },
        );
    }
    let entry_loc = f.loc.first().copied();
    let entry_via = f.via.first().copied();
    let mut block_of = vec![0usize; f.code.len()];
    for (b, blk) in f.blocks.values().iter().enumerate() {
        block_of[blk.start as usize..blk.end as usize].fill(b);
    }
    // The bounds of `r` at `pc`: the `MakeDope` runs before every path to the reader.
    let at = |r: Reg, pc: usize| -> Option<(Reg, Reg, Reg)> {
        let m = known.get(&r)?;
        let b = block_of[pc];
        let reach = if b == m.block {
            pc >= m.from
        } else {
            dom[b][m.block / 64] >> (m.block % 64) & 1 == 1
        };
        reach.then_some(m.bounds)
    };
    // The one `Load` of each layout read for its size.
    let mut size_load: FxMap<Reg, usize> = FxMap::default();
    for (pc, i) in f.code.iter().enumerate() {
        if let Inst::Load {
            addr, w: Width::B8, ..
        } = *i
            && uses.get(addr.0 as usize) == Some(&1)
            && defs.get(addr.0 as usize) == Some(&1)
        {
            size_load.insert(addr, pc);
        }
    }
    let n = f.code.len();
    let mut code = Vec::with_capacity(n + 8);
    let mut loc = Vec::with_capacity(n + 8);
    let mut via = Vec::with_capacity(if f.via.is_empty() { 0 } else { n + 8 });
    let mut ranges = Vec::with_capacity(f.blocks.len());
    let mut any = false;
    // For a layout whose size is worked out at its `Load`: the length and the form.
    let mut sized: FxMap<Reg, (Reg, bool, u64)> = FxMap::default();
    let mut reg_ty = std::mem::take(&mut f.reg_ty);
    let mut fresh = |t: RegTy| {
        reg_ty.push(t);
        Reg(reg_ty.len() as u32 - 1)
    };
    let blocks: Vec<(u32, u32)> = f.blocks.values().iter().map(|b| (b.start, b.end)).collect();
    // The lengths made so far by bounds: the register and its block, in the order made.
    let mut lens: FxMap<(Reg, Reg, Reg), Vec<(Reg, usize)>> = FxMap::default();
    let mut out: Vec<Inst> = Vec::new();
    // The constants of registers defined once.
    let consts: FxMap<Reg, i64> = f
        .code
        .iter()
        .filter_map(|i| match *i {
            Inst::Const { dst, v } if defs.get(dst.0 as usize) == Some(&1) => Some((dst, v)),
            _ => None,
        })
        .collect();
    for (bk, (start, end)) in blocks.into_iter().enumerate() {
        let first = code.len() as u32;
        if bk == 0 {
            for &i in &entry {
                code.push(i);
                loc.extend(entry_loc);
                via.extend(entry_via);
            }
        }
        for pc in start as usize..end as usize {
            let i = f.code[pc];
            out.clear();
            match i {
                Inst::DopeLen { dst, dope, dim: 0 } if at(dope, pc).is_some() => {
                    let (left, right, dir) = at(dope, pc).unwrap_or((dope, dope, dope));
                    out.push(Inst::BoundsLen {
                        dst,
                        left,
                        right,
                        dir,
                    });
                    if defs.get(dst.0 as usize) == Some(&1) {
                        lens.entry((left, right, dir)).or_default().push((dst, bk));
                    }
                }
                Inst::Pos {
                    dst,
                    dope,
                    dim: 0,
                    idx,
                    what,
                } if regs && at(dope, pc).is_some() => {
                    let (left, right, dir) = at(dope, pc).unwrap_or((dope, dope, dope));
                    out.push(Inst::BoundsPos {
                        dst,
                        left,
                        right,
                        dir,
                        idx,
                        what,
                    });
                }
                Inst::SlicePos {
                    dst,
                    dope,
                    left,
                    right,
                    dir,
                    what,
                    dir_what,
                } if regs && at(dope, pc).is_some() => {
                    let (al, ar, ad) = at(dope, pc).unwrap_or((dope, dope, dope));
                    let start = f.operands.len() as u32;
                    f.operands.extend([al, ar, ad, left, right, dir]);
                    out.push(Inst::BoundsSlice {
                        dst,
                        regs: crate::Regs { start, len: 6 },
                        what,
                        dir_what,
                    });
                }
                Inst::SubtypeCheck {
                    dope,
                    want,
                    dims: 1,
                    what,
                } if (at(dope, pc).is_some() || at(want, pc).is_some())
                    && prog.checks.get(what).is_some_and(|c| c.subtype.is_empty()) =>
                {
                    let (x, y) = (at(dope, pc), at(want, pc));
                    if x.is_none() || x != y {
                        let doms = |b: usize| dom[bk][b / 64] >> (b % 64) & 1 == 1;
                        let mut len = |d: Reg, b: Option<(Reg, Reg, Reg)>, out: &mut Vec<Inst>| {
                            if let Some(b) = b
                                && let Some(&(r, _)) = lens
                                    .get(&b)
                                    .and_then(|v| v.iter().find(|l| l.1 == bk || doms(l.1)))
                            {
                                return r;
                            }
                            let dst = fresh(RegTy::I64);
                            out.push(match b {
                                Some((left, right, dir)) => Inst::BoundsLen {
                                    dst,
                                    left,
                                    right,
                                    dir,
                                },
                                None => Inst::DopeLen {
                                    dst,
                                    dope: d,
                                    dim: 0,
                                },
                            });
                            dst
                        };
                        let a = len(dope, x, &mut out);
                        let b = len(want, y, &mut out);
                        out.push(Inst::Check {
                            kind: CheckKind::Length,
                            v: a,
                            lo: b,
                            hi: b,
                            what,
                        });
                    }
                }
                Inst::LayoutOf { dst, ty, desc, .. }
                    if at(desc, pc).is_some()
                        && defs.get(dst.0 as usize) == Some(&1)
                        && size_load.contains_key(&dst)
                        && size_form(prog, ty).is_some() =>
                {
                    let (left, right, dir) = at(desc, pc).unwrap_or((desc, desc, desc));
                    let (planes, stride) = size_form(prog, ty).unwrap_or((false, 1));
                    let doms = |b: usize| dom[bk][b / 64] >> (b % 64) & 1 == 1;
                    let had = lens
                        .get(&(left, right, dir))
                        .and_then(|v| v.iter().find(|l| l.1 == bk || doms(l.1)))
                        .map(|l| l.0);
                    let len = had.unwrap_or_else(|| {
                        let len = fresh(RegTy::I64);
                        out.push(Inst::BoundsLen {
                            dst: len,
                            left,
                            right,
                            dir,
                        });
                        lens.entry((left, right, dir)).or_default().push((len, bk));
                        len
                    });
                    sized.insert(dst, (len, planes, stride));
                }
                Inst::Load { dst, addr, .. } if sized.contains_key(&addr) => {
                    let (len, planes, stride) = sized.remove(&addr).unwrap_or((addr, false, 1));
                    let mut k = |v: i64, out: &mut Vec<Inst>| {
                        let r = fresh(RegTy::I64);
                        out.push(Inst::Const { dst: r, v });
                        r
                    };
                    if planes {
                        let (k63, k64, k32) = (k(63, &mut out), k(64, &mut out), k(32, &mut out));
                        let (t1, t2) = (fresh(RegTy::I64), fresh(RegTy::I64));
                        out.push(Inst::Bin {
                            op: IntOp::Add,
                            dst: t1,
                            a: len,
                            b: k63,
                        });
                        out.push(Inst::Bin {
                            op: IntOp::Div,
                            dst: t2,
                            a: t1,
                            b: k64,
                        });
                        out.push(Inst::Bin {
                            op: IntOp::Mul,
                            dst,
                            a: t2,
                            b: k32,
                        });
                    } else {
                        let ks = k(stride as i64, &mut out);
                        out.push(Inst::Bin {
                            op: IntOp::Mul,
                            dst,
                            a: len,
                            b: ks,
                        });
                    }
                }
                Inst::Copy { dst, src, bytes }
                    if at(src, pc).is_some() && consts.get(&bytes) == Some(&24) =>
                {
                    let (l, r, d) = at(src, pc).unwrap_or((src, src, src));
                    let start = f.operands.len() as u32;
                    f.operands.extend([l, r, d]);
                    out.push(Inst::MakeDope {
                        at: dst,
                        parts: crate::Regs { start, len: 3 },
                    });
                }
                _ => out.push(i),
            }
            any |= out.len() != 1 || out[0] != i;
            for &x in &out {
                code.push(x);
                loc.push(f.loc[pc]);
                if let Some(&v) = f.via.get(pc) {
                    via.push(v);
                }
            }
        }
        ranges.push((first, code.len() as u32));
    }
    f.reg_ty = reg_ty;
    if !any {
        return false;
    }
    for (b, (s, e)) in ranges.into_iter().enumerate() {
        if let Some(blk) = f.blocks.get_mut(crate::BlockId(b as u32)) {
            blk.start = s;
            blk.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
    true
}

/// The offset in `rest` of the `TakeBounds` a `RetBounds` of the registers `from` before it answers:
/// nothing between but constants, addresses and moves that leave those registers alone.
fn taken_by(rest: &[Inst], from: [Reg; 3]) -> Option<usize> {
    for (q, i) in rest.iter().enumerate() {
        match *i {
            Inst::TakeBounds { .. } => return Some(q),
            Inst::Const { dst, .. } | Inst::AddrOf { dst, .. } | Inst::Move { dst, .. }
                if !from.contains(&dst) => {}
            _ => return None,
        }
    }
    None
}

/// A result spliced into its caller (the read of a result table, before the first pass): the bounds
/// go to the registers of the `TakeBounds`, so the walk sees what is known of them.
pub(super) fn fold_takes(f: &mut crate::Func) {
    let n = f.code.len();
    let mut code = Vec::with_capacity(n);
    let mut loc = Vec::with_capacity(n);
    let mut via = Vec::with_capacity(if f.via.is_empty() { 0 } else { n });
    let mut taken: FxMap<usize, (Reg, Reg, Reg)> = FxMap::default();
    let ranges: Vec<(u32, u32)> = f.blocks.values().iter().map(|b| (b.start, b.end)).collect();
    for (b, (start, end)) in ranges.into_iter().enumerate() {
        let first = code.len() as u32;
        for pc in start as usize..end as usize {
            let i = f.code[pc];
            let out: Vec<Inst> = match i {
                Inst::RetBounds { left, right, dir }
                    if let Some(q) =
                        taken_by(&f.code[pc + 1..end as usize], [left, right, dir]) =>
                {
                    taken.insert(pc + 1 + q, (left, right, dir));
                    Vec::new()
                }
                Inst::TakeBounds { left, right, dir } if taken.contains_key(&pc) => {
                    let (l, r, d) = taken[&pc];
                    vec![
                        Inst::Move { dst: left, src: l },
                        Inst::Move { dst: right, src: r },
                        Inst::Move { dst: dir, src: d },
                    ]
                }
                _ => vec![i],
            };
            for x in out {
                code.push(x);
                loc.push(f.loc[pc]);
                via.extend(f.via.get(pc).copied());
            }
        }
        if let Some(blk) = f.blocks.get_mut(crate::BlockId(b as u32)) {
            blk.start = first;
            blk.end = code.len() as u32;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
}
