//! Copies of subprograms by the descriptors of their arguments (prototype).
//!
//! A descriptor a call passes can be a design fact the callee does not see. The last step of a
//! copy's optimisation asks, per `Call`, which `'dope` arguments have bytes it knows and turns the
//! call into one of the copy of the callee for those bytes (`Program::sub_copy`, `copies.rs`). The
//! copy has them as `Opt::wf` facts (`Opt::param_facts`), so its lengths, positions, layouts and
//! checks fold as those of a process copy do, and its own calls are asked the same.
//!
//! The target is rewritten last: every pass before it reads callees by id, and a copy's own slot
//! is empty. Nothing here assumes more than bytes: the callee still gets the pointer.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::{AddrSpace, Func, FuncFlags, FuncId, FuncKind, Inst, Program, Reg, Ty, Width};

use super::InstView;
use super::decl::Facts;

/// `YAVS_NO_SUBCOPY=1` turns them off (fingerprint `subcopy`).
pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_SUBCOPY")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// At most this many copies of one subprogram (`YAVS_SUB_PER`).
pub(crate) fn per_function() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| env_usize("YAVS_SUB_PER", 8))
}

/// The most descriptor bytes of one parameter a key takes.
const MAX_DESC: u64 = 4096;
/// Block visits that find what holds at the block starts, at least; past the limit nothing is rewritten.
const MAX_VISITS: usize = 4096;

/// A subprogram that can have copies: called by id, no frame above it to reach, no suspension.
fn copyable(f: &Func) -> bool {
    matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && !f.spare
        && !f.nested
        && f.parent.is_none()
        && !f.flags.contains(FuncFlags::MAY_SUSPEND)
}

/// Slots to reserve for copies of subprograms: four per subprogram that takes such a descriptor
/// (`YAVS_SUB_SLOTS` is the most, 1024).
pub(crate) fn slots(prog: &Program) -> usize {
    if !enabled() {
        return 0;
    }
    let n = prog
        .funcs
        .values()
        .iter()
        .filter(|f| {
            copyable(f)
                && f.params
                    .iter()
                    .any(|p| p.arr.is_some_and(|t| worth_keying(prog, t)))
        })
        .count();
    n.saturating_mul(4).min(env_usize("YAVS_SUB_SLOTS", 1024))
}

/// Register `r` is read: a descriptor read counts in `out`; the frame address it holds, if any.
fn note(
    from: &HashMap<u32, u32>,
    at: &HashMap<u32, i64>,
    r: Reg,
    out: &mut Vec<u32>,
) -> Option<i64> {
    if let Some(&s) = from.get(&r.0) {
        out.push(s);
    }
    at.get(&r.0).copied()
}

/// The `'dope` slots of `g` a copy may key on: its lowered code reads the descriptor, so knowing it
/// can fold something and keying on any other would only part instances; and nothing writes the
/// slot or lets a pointer to the parameters out, so what the caller passed stays there.
pub(crate) fn reads(g: &Func) -> Vec<u32> {
    let raw = g.full();
    let params_end = raw
        .params
        .iter()
        .map(|p| i64::from(p.slot) + i64::from(p.width.bytes()))
        .max()
        .unwrap_or(0);
    let is_dope = |o: i64| {
        raw.params
            .iter()
            .any(|p| i64::from(p.slot) == o && p.name.ends_with("'dope"))
    };
    // The lowered code defines its addresses once, before their uses in the order of the code.
    let (mut at, mut k, mut from) = (HashMap::new(), HashMap::new(), HashMap::new());
    let mut out = Vec::new();
    let mut written = Vec::new();
    for i in &raw.code {
        let mut leaks = false;
        match *i {
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } => {
                at.insert(dst.0, i64::from(off));
            }
            Inst::Const { dst, v } => {
                k.insert(dst.0, v);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                if let Some(&s) = from.get(&base.0) {
                    from.insert(dst.0, s);
                } else if let Some(&o) = at.get(&base.0) {
                    match k.get(&pos.0) {
                        Some(&p) => {
                            at.insert(dst.0, o + p * i64::from(stride));
                        }
                        None => leaks |= o < params_end,
                    }
                }
            }
            Inst::Move { dst, src } => {
                if let Some(&s) = from.get(&src.0) {
                    from.insert(dst.0, s);
                }
                if let Some(&o) = at.get(&src.0) {
                    at.insert(dst.0, o);
                }
            }
            Inst::Load { dst, addr, .. } => {
                note(&from, &at, addr, &mut out);
                if let Some(&o) = at.get(&addr.0)
                    && is_dope(o)
                {
                    from.insert(dst.0, o as u32);
                }
            }
            Inst::Store { addr, src, w } => {
                if let Some(&o) = at.get(&addr.0) {
                    written.push((o, i64::from(w.bytes())));
                }
                note(&from, &at, addr, &mut out);
                leaks |= note(&from, &at, src, &mut out).is_some_and(|o| o < params_end);
            }
            _ => {
                i.visit(&mut |r, role, _| {
                    if role == crate::Role::Use {
                        leaks |= note(&from, &at, r, &mut out).is_some_and(|o| o < params_end);
                    }
                });
                if let Some(l) = i.list() {
                    for &r in raw.regs(l).unwrap_or(&[]) {
                        leaks |= note(&from, &at, r, &mut out).is_some_and(|o| o < params_end);
                    }
                }
            }
        }
        if leaks {
            return Vec::new();
        }
    }
    out.sort_unstable();
    out.dedup();
    out.retain(|&s| {
        !written
            .iter()
            .any(|&(o, n)| o < i64::from(s) + 8 && i64::from(s) < o + n)
    });
    out
}

fn encode(out: &mut Vec<u8>, slot: u32, bytes: &[u8]) {
    out.extend_from_slice(&slot.to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// The (slot, bytes) pairs of a copy's `data`.
pub(crate) fn decode(mut d: &[u8]) -> Vec<(u32, &[u8])> {
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let mut out = Vec::new();
    while d.len() >= 8 {
        let (slot, n) = (word(d), word(&d[4..]) as usize);
        out.push((slot, &d[8..8 + n]));
        d = &d[8 + n..];
    }
    out
}

/// A type whose layout compiled code cannot make inline from a length: a record, or an array of
/// composites. Knowing such a descriptor removes callbacks and steps and lets the subprogram run on
/// native frames; knowing one of a plain vector removes an arithmetic step or two and splits the
/// code of the subprogram, which the first compile has to pay again.
fn worth_keying(prog: &Program, ty: crate::TyId) -> bool {
    static ALL: OnceLock<bool> = OnceLock::new();
    if *ALL.get_or_init(|| std::env::var_os("YAVS_SUB_ALL").is_some()) {
        return true;
    }
    match prog.types.get(ty) {
        Some(Ty::Record { .. }) => true,
        Some(Ty::Array { elem, dims, .. }) => {
            *dims > 1
                || matches!(
                    prog.types.get(*elem),
                    Some(Ty::Record { .. } | Ty::Array { .. })
                )
        }
        _ => false,
    }
}

/// What a register is known to point at.
#[derive(Clone, PartialEq)]
enum At {
    /// An address in read-only memory of the view.
    Addr(AddrSpace, u32),
    /// `off` bytes into known bytes: a parameter's descriptor, a heap object's.
    Bytes(Rc<[u8]>, u32),
    /// An address in the frame.
    Slot(u32),
}

/// What registers defined more than once hold where a block starts; a block no path reached has
/// none.
#[derive(Clone, Default, PartialEq)]
struct State {
    at: HashMap<u32, At>,
    k: HashMap<u32, i64>,
}

impl State {
    /// What holds on both paths; whether that is less than `self` held.
    fn meet(&mut self, o: &State) -> bool {
        let n = self.at.len() + self.k.len();
        self.at.retain(|r, a| o.at.get(r) == Some(a));
        self.k.retain(|r, v| o.k.get(r) == Some(v));
        n != self.at.len() + self.k.len()
    }
}

struct Walk<'a> {
    prog: &'a Program,
    view: InstView<'a>,
    /// What the declarative part of a process left in its frame; a copy of a subprogram has its
    /// `'dope` slots here.
    facts: Option<&'a Facts>,
    /// Registers defined once whose value holds wherever they are read, from other such registers
    /// alone (`once_at`, `once_k`, found first, wherever they are defined).
    once: Vec<bool>,
    once_at: HashMap<u32, At>,
    once_k: HashMap<u32, i64>,
    /// Finding those: the order of the code is not the order it runs.
    seeding: bool,
    st: State,
}

impl Walk<'_> {
    fn at_of(&self, r: Reg) -> Option<&At> {
        self.once_at.get(&r.0).or_else(|| self.st.at.get(&r.0))
    }

    fn k_of(&self, r: Reg) -> Option<i64> {
        self.once_k
            .get(&r.0)
            .or_else(|| self.st.k.get(&r.0))
            .copied()
    }

    fn step(&mut self, i: &Inst) {
        let (at, k) = match *i {
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } => (Some((dst, At::Slot(off))), None),
            Inst::AddrOf { dst, space, off } => (Some((dst, At::Addr(space, off))), None),
            Inst::Const { dst, v } => (None, Some((dst, v))),
            Inst::Move { dst, src } => (
                self.at_of(src).map(|a| (dst, a.clone())),
                self.k_of(src).map(|v| (dst, v)),
            ),
            Inst::Load {
                dst,
                addr,
                w: Width::B8,
            } => {
                let a = match self.at_of(addr) {
                    Some(&At::Slot(s)) => self.facts.and_then(|f| {
                        if let Some((b, o)) = f.descs.get(&s) {
                            return Some((dst, At::Bytes(b.clone(), *o)));
                        }
                        f.addrs.get(&s).map(|&(sp, o)| match sp {
                            AddrSpace::Frame => (dst, At::Slot(o)),
                            _ => (dst, At::Addr(sp, o)),
                        })
                    }),
                    _ => None,
                };
                (a, None)
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let d = self
                    .k_of(pos)
                    .and_then(|p| p.checked_mul(i64::from(stride)))
                    .and_then(|d| u32::try_from(d).ok());
                let a = match (self.at_of(base), d) {
                    (Some(&At::Addr(s, o)), Some(d)) => {
                        o.checked_add(d).map(|o| (dst, At::Addr(s, o)))
                    }
                    (Some(&At::Slot(o)), Some(d)) => o.checked_add(d).map(|o| (dst, At::Slot(o))),
                    (Some(At::Bytes(b, o)), Some(d)) => {
                        o.checked_add(d).map(|o| (dst, At::Bytes(b.clone(), o)))
                    }
                    _ => None,
                };
                (a, None)
            }
            // Nothing but what a frame starts with lives across a suspension.
            Inst::Suspend { .. } => {
                self.st = State::default();
                (None, None)
            }
            _ => (None, None),
        };
        // Whatever it defines is no longer what it was.
        let st = &mut self.st;
        super::for_defs(i, |r| {
            st.at.remove(&r.0);
            st.k.remove(&r.0);
        });
        // Only seeding speaks for every path; on a walk a value is what this path knows of it, even
        // in a register defined once (a `Move` of a register that differs by path).
        let once = |r: Reg| self.once.get(r.0 as usize) == Some(&true);
        if let Some((r, a)) = at {
            if !self.seeding {
                if !self.once_at.contains_key(&r.0) {
                    self.st.at.insert(r.0, a);
                }
            } else if once(r) {
                self.once_at.insert(r.0, a);
            }
        }
        if let Some((r, v)) = k {
            if !self.seeding {
                if !self.once_k.contains_key(&r.0) {
                    self.st.k.insert(r.0, v);
                }
            } else if once(r) {
                self.once_k.insert(r.0, v);
            }
        }
    }

    /// The `n` bytes at what `r` points at, when they cannot change: pool constants, instance and
    /// package data (frozen before any process runs), a known descriptor.
    fn bytes(&self, r: Reg, n: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; n];
        let (data, holes, pkgs, _) = self.view;
        match *self.at_of(r)? {
            At::Slot(o) => {
                let b = self.facts?.bytes.get(o as usize..o as usize + n)?;
                for (d, s) in out.iter_mut().zip(b) {
                    *d = (*s)?;
                }
            }
            At::Bytes(ref b, o) => {
                out.copy_from_slice(b.get(o as usize..o as usize + n)?);
            }
            At::Addr(AddrSpace::Const(c), o) => {
                if !self.prog.consts.get(c)?.read(o as usize, &mut out) {
                    return None;
                }
            }
            At::Addr(AddrSpace::Inst, o) => {
                let (a, e) = (o as usize, o as usize + n);
                if holes
                    .iter()
                    .any(|&(h, z)| (h as usize) < e && a < (h + z) as usize)
                {
                    return None;
                }
                out.copy_from_slice(data.get(a..e)?);
            }
            At::Addr(AddrSpace::Pkg(p), o) => {
                let d = pkgs.iter().find(|q| q.0 == p)?.1;
                out.copy_from_slice(d.get(o as usize..o as usize + n)?);
            }
            At::Addr(..) => return None,
        }
        Some(out)
    }

    /// The key of the call of `g` with `regs`, when some descriptor is known.
    fn key(&self, func: FuncId, g: &Func, regs: &[Reg]) -> Option<Vec<u8>> {
        if regs.len() != g.params.len() {
            return None;
        }
        let mut data = Vec::new();
        let reads = self.prog.sub_reads(func, g);
        for (j, p) in g.params.iter().enumerate().skip(1) {
            // The data cell of an array or record is followed by its descriptor.
            let Some(ty) = g.params[j - 1].arr else {
                continue;
            };
            if !reads.contains(&p.slot) || !worth_keying(self.prog, ty) {
                continue;
            }
            let Some(n) = Ty::desc_size(&self.prog.types, ty).filter(|&n| n > 0) else {
                continue;
            };
            if n > MAX_DESC {
                self.prog.note_sub_big();
                continue;
            }
            if let Some(b) = self.bytes(regs[j], n as usize) {
                encode(&mut data, p.slot, &b);
            }
        }
        (!data.is_empty()).then_some(data)
    }
}

/// Call the copies of the callees for what this copy knows of their arguments.
pub(super) fn rewrite_calls(
    prog: &Program,
    f: &mut Func,
    view: InstView<'_>,
    facts: Option<&Facts>,
) {
    if !enabled() || !super::specialise_on() {
        return;
    }
    let mut defs = vec![0u8; f.reg_ty.len()];
    for i in &f.code {
        super::for_defs(i, |r| {
            if let Some(d) = defs.get_mut(r.0 as usize) {
                *d = d.saturating_add(1);
            }
        });
    }
    let once: Vec<bool> = defs.iter().map(|&d| d == 1).collect();
    let mut w = Walk {
        prog,
        view,
        facts,
        once,
        once_at: HashMap::new(),
        once_k: HashMap::new(),
        seeding: true,
        st: State::default(),
    };
    for _ in 0..3 {
        for i in &f.code {
            w.step(i);
        }
    }
    w.seeding = false;
    // What holds where each block starts: every path agrees (a must analysis, from "all" down).
    let succ = super::memo::successors(prog, f);
    let blocks: Vec<(usize, usize)> = f
        .blocks
        .values()
        .iter()
        .map(|b| (b.start as usize, b.end as usize))
        .collect();
    let mut start: Vec<Option<State>> = vec![None; blocks.len()];
    if let Some(s) = start.first_mut() {
        *s = Some(State::default());
    }
    // Each block's start only ever holds less, so this ends; the limit is for the compile time.
    let mut dirty = vec![0usize];
    let mut budget = MAX_VISITS.max(blocks.len().saturating_mul(32));
    while let Some(b) = dirty.pop() {
        if budget == 0 {
            prog.note_sub_gave_up();
            return;
        }
        budget -= 1;
        let Some(st) = start[b].clone() else {
            continue;
        };
        w.st = st;
        for i in &f.code[blocks[b].0..blocks[b].1] {
            w.step(i);
        }
        for &t in &succ[b] {
            let changed = match &mut start[t] {
                Some(s) => s.meet(&w.st),
                s => {
                    *s = Some(w.st.clone());
                    true
                }
            };
            if changed {
                dirty.push(t);
            }
        }
    }
    let mut todo: Vec<(usize, FuncId)> = Vec::new();
    for (b, st) in start.into_iter().enumerate() {
        let Some(st) = st else { continue };
        w.st = st;
        for pc in blocks[b].0..blocks[b].1 {
            let i = &f.code[pc];
            if let Inst::Call { func, args, .. } = *i
                && let Some(g) = prog.funcs.get(func)
                && copyable(g)
                && let Some(regs) = f.regs(args)
                && let Some(data) = w.key(func, g, regs)
                && let Some(id) = prog.sub_copy(func, &data)
            {
                todo.push((pc, id));
            }
            w.step(i);
        }
    }
    for (pc, id) in todo {
        if let Inst::Call { func, .. } = &mut f.code[pc] {
            *func = id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConstData, ConstId, ProgramBuilder, RegTy, Regs, Ty, opt};

    fn span() -> yavs_diag::Span {
        yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1)
    }

    /// The descriptors of a two-dimensional array: `(left, right, direction)` per dimension.
    fn desc(left: u8) -> Vec<u8> {
        let mut d = vec![0u8; 48];
        d[0] = left;
        d[16] = 1;
        d[24] = 1;
        d[40] = 1;
        d
    }

    /// `f(x : array)`: its length, or (`write`) a store to the slot of the descriptor first, or
    /// (`unread`) nothing about the descriptor.
    fn callee(pb: &mut ProgramBuilder, write: bool, unread: bool) -> FuncId {
        let elem = pb.ty(Ty::Int { lo: 0, hi: 9 });
        let t = pb.ty(Ty::Array {
            elem,
            elem_desc: None,
            dims: 2,
        });
        let f = pb.declare("f", FuncKind::Function, None);
        let mut fb = pb.define(f);
        fb.stmt(crate::StmtKind::Wait, span());
        let x = fb.param(
            "x",
            Width::B8,
            crate::ParamClass::Value,
            crate::Mode::In,
            None,
        );
        fb.param_array(x, t);
        let d = fb.param(
            "x'dope",
            Width::B8,
            crate::ParamClass::Value,
            crate::Mode::In,
            None,
        );
        let a = fb.addr(AddrSpace::Frame, d);
        let v = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: v,
            addr: a,
            w: Width::B8,
        });
        if write {
            fb.emit(Inst::Store {
                addr: a,
                src: v,
                w: Width::B8,
            });
        }
        let n = fb.reg(RegTy::I64);
        if unread {
            fb.emit(Inst::Const { dst: n, v: 1 });
        } else {
            fb.emit(Inst::DopeLen {
                dst: n,
                dope: v,
                dim: 0,
            });
        }
        fb.emit(Inst::Ret { v: Some(n) });
        fb.finish();
        f
    }

    /// `g`: a descriptor by the branch taken (`first` or `second`), then `f` of it, through a
    /// `Move` to a register defined once when `moved`.
    fn caller(
        pb: &mut ProgramBuilder,
        f: FuncId,
        first: ConstId,
        second: ConstId,
        moved: bool,
    ) -> FuncId {
        let g = pb.declare("g", FuncKind::Function, None);
        let mut fb = pb.define(g);
        fb.stmt(crate::StmtKind::Wait, span());
        let (y, n, j) = (fb.block(), fb.block(), fb.block());
        let c = fb.int(1);
        fb.emit(Inst::CondBr { c, t: y, f: n });
        let d = fb.reg(RegTy::Ptr);
        for (b, k) in [(y, first), (n, second)] {
            fb.at(b);
            fb.emit(Inst::AddrOf {
                dst: d,
                space: AddrSpace::Const(k),
                off: 0,
            });
            fb.emit(Inst::Br { to: j });
        }
        fb.at(j);
        let arg = if moved {
            let m = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Move { dst: m, src: d });
            m
        } else {
            d
        };
        let data = fb.addr(AddrSpace::Const(first), 0);
        let args: Regs = fb.regs(&[data, arg]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: f,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
        g
    }

    fn target(p: &Program, g: FuncId, facts: Option<&Facts>, view: InstView<'_>) -> FuncId {
        let mut f = p.funcs.get(g).expect("the caller").full().clone();
        rewrite_calls(p, &mut f, view, facts);
        f.code
            .iter()
            .find_map(|i| match *i {
                Inst::Call { func, .. } => Some(func),
                _ => None,
            })
            .expect("a call")
    }

    const NONE: InstView<'static> = (&[], &[], &[], &[]);

    fn built(same: bool, moved: bool, write: bool, unread: bool) -> (Program, FuncId, FuncId) {
        let mut pb = ProgramBuilder::new();
        let f = callee(&mut pb, write, unread);
        let a = pb.konst(ConstData::Bytes(desc(3)));
        let b = pb.konst(ConstData::Bytes(desc(7)));
        let g = caller(&mut pb, f, a, if same { a } else { b }, moved);
        let mut p = pb.finish().expect("a program");
        p.reserve_copies(1);
        (p, f, g)
    }

    /// A descriptor that differs by path is not known, however it reaches the call: a register
    /// defined once takes what the register it copies holds on the path, not on the first path.
    #[test]
    fn a_join_of_two_descriptors_is_no_fact() {
        if !opt::specialise_on() {
            return;
        }
        for moved in [false, true] {
            let (p, f, g) = built(false, moved, false, false);
            assert_eq!(target(&p, g, None, NONE), f, "moved {moved}");
            assert_eq!(p.copy_stats().subs, 0);
        }
    }

    /// The same descriptor on both paths is known; the copy is keyed on its bytes, and is one copy.
    #[test]
    fn a_join_of_one_descriptor_is_a_fact() {
        if !opt::specialise_on() {
            return;
        }
        for moved in [false, true] {
            let (p, f, g) = built(true, moved, false, false);
            let t = target(&p, g, None, NONE);
            assert_ne!(t, f, "moved {moved}");
            assert_eq!(p.base_of(t), f);
            let params = p.sub_copy_params(t);
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].1, desc(3));
            assert_eq!(target(&p, g, None, NONE), t);
            assert_eq!(p.copy_stats().subs, 1);
            // The copy knows the length; the subprogram still asks.
            let kind = |id| {
                p.func(id)
                    .expect("code")
                    .code
                    .iter()
                    .any(|i| matches!(i, Inst::DopeLen { .. }))
            };
            assert!(
                !kind(t),
                "the length of the descriptor did not fold: {}",
                p.dump_func(t)
            );
        }
    }

    /// A slot the body writes is no fact: no copy is keyed on it, even if a caller knew it.
    #[test]
    fn a_written_descriptor_slot_is_not_trusted() {
        if !opt::specialise_on() {
            return;
        }
        let (p, f, g) = built(true, false, true, false);
        assert_eq!(target(&p, g, None, NONE), f);
        assert_eq!(p.copy_stats().subs, 0);
    }

    /// A descriptor the callee never asks about makes no copy.
    #[test]
    fn a_descriptor_nothing_reads_makes_no_copy() {
        if !opt::specialise_on() {
            return;
        }
        let (p, f, g) = built(true, false, false, true);
        assert_eq!(target(&p, g, None, NONE), f);
        assert_eq!(p.copy_stats().subs, 0);
    }

    /// Bytes under a `for` generate's parameter are not folded by a process copy, and not keyed.
    #[test]
    fn a_hole_of_the_instance_data_is_no_fact() {
        if !opt::specialise_on() {
            return;
        }
        let mut pb = ProgramBuilder::new();
        let f = callee(&mut pb, false, false);
        let g = pb.declare("g", FuncKind::Function, None);
        let mut fb = pb.define(g);
        fb.stmt(crate::StmtKind::Wait, span());
        let d = fb.addr(AddrSpace::Inst, 8);
        let args = fb.regs(&[d, d]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: f,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
        let mut p = pb.finish().expect("a program");
        p.reserve_copies(1);
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&desc(3));
        let hole = [(16u32, 8u32)];
        assert_eq!(target(&p, g, None, (&data, &hole, &[], &[])), f);
        let t = target(&p, g, None, (&data, &[], &[], &[]));
        assert_ne!(t, f);
        // Out of the data: nothing to key on either.
        assert_eq!(target(&p, g, None, (&data[..30], &[], &[], &[])), f);
    }

    /// The most copies of one subprogram; a limit of one is one copy for two descriptors.
    #[test]
    fn slots_for_copies_do_not_take_those_of_processes() {
        if !opt::specialise_on() {
            return;
        }
        let (p, f, _) = built(true, false, false, false);
        let st = p.copy_stats();
        assert_eq!(st.reserved, 1 + slots(&p));
        // Past the limit of one function: `None`, so the call stays a call of `f`.
        for k in 0..per_function() {
            if p.copy_stats().subs < st.reserved - 1 {
                let mut d = Vec::new();
                encode(&mut d, 8, &[k as u8; 8]);
                assert!(p.sub_copy(f, &d).is_some());
            }
        }
        let mut d = Vec::new();
        encode(&mut d, 8, &[200; 8]);
        assert!(p.sub_copy(f, &d).is_none());
        assert!(p.copy_stats().copies < st.reserved);
    }
}

#[cfg(test)]
mod worth {
    use super::*;
    use crate::{ProgramBuilder, Ty};

    /// Only descriptors whose layout compiled code cannot make from a length are keyed on.
    #[test]
    fn a_plain_vector_is_not_worth_a_copy() {
        if std::env::var_os("YAVS_SUB_ALL").is_some() {
            return;
        }
        let mut pb = ProgramBuilder::new();
        let int = pb.ty(Ty::Int { lo: 0, hi: 9 });
        let plain = pb.ty(Ty::Array {
            elem: int,
            elem_desc: None,
            dims: 1,
        });
        let matrix = pb.ty(Ty::Array {
            elem: int,
            elem_desc: None,
            dims: 2,
        });
        let nested = pb.ty(Ty::Array {
            elem: plain,
            elem_desc: None,
            dims: 1,
        });
        let p = pb.finish().expect("a program");
        assert!(!worth_keying(&p, plain));
        assert!(worth_keying(&p, matrix));
        assert!(worth_keying(&p, nested));
    }
}
