//! `Inst::SigGather` and `Inst::SigScatter` (#677): a record signal, or an array of records, read
//! into, or driven from, a value in one instruction, instance by instance and leaf by leaf in a
//! Rust loop over the leaves of a plan (`yavs_ir::gather`). Each leaf does what the
//! `SigRead`/`SigReadScalar` and `Drive` of the lowered per-leaf code did, minus the instruction
//! dispatch, the staging buffers and the register traffic. A plan is made from the leaf table and
//! the signal's descriptor the first time and kept by the machine that met it (no lock or shared
//! reference count on the hot path, #677); the runtime keeps one copy for the machines that
//! come later, so a record with bounds from generics costs one comparison of its descriptor.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use yavs_ir::gather::{self, Leaf, LeafKind, Plan, PlanError};
use yavs_ir::{
    ConstData, ConstId, DriveMode, Func, Part, Program, Reg, Regs, SCATTER_PART, SCATTER_REJECT, Ty,
};
use yavs_kernel::{Ctx, Elem, Range};

use crate::InstSig;
use crate::exec::{CONST, Machine, R, cell, cold, ice, rt_err, split, to_time};

/// The plans of a runtime: per leaf table, the descriptor size of its value type and the plans
/// made so far with the descriptor bytes they are for. A machine copies the ones it uses
/// ([`MPlan`]); this is where the machines that meet a table and descriptor later find the plan.
#[derive(Default)]
pub(crate) struct Plans(RwLock<HashMap<u32, Known>>);

/// No constant-space descriptor address yet ([`MPlan::at`]): address 0 is frame space.
const NO_ADDR: u64 = u64::MAX;

/// The descriptor size of a table's value type and the plans made for it, by descriptor bytes.
type Known = (u64, Vec<(Box<[u8]>, Arc<Plan>)>);

/// A plan of a machine's own: the table and descriptor size it is for, the descriptor bytes, and
/// the plan with the instance signal of each leaf. `bound` is taken out while an instruction uses
/// it (no lock, no shared reference count; boxed, so taking it moves one pointer). `at` is the
/// constant-space address of a descriptor already found equal to `desc`: constants never change,
/// so a descriptor at that address needs no comparison.
pub(crate) struct MPlan {
    tab: u32,
    ds: u64,
    at: u64,
    desc: Box<[u8]>,
    bound: Option<Box<Bound>>,
}

/// A plan and the instance signal of each of its leaves; for a whole value also the instances of
/// its leaves (`items`) in groups read from one plane word (`reads`) and driven as one run
/// (`runs`), found when the plan is bound.
pub(crate) struct Bound {
    plan: Arc<Plan>,
    sigs: Box<[InstSig]>,
    items: Box<[Item]>,
    reads: Box<[Grp]>,
    runs: Box<[Grp]>,
}

/// One instance of a leaf in a whole value: the leaf, its byte offset in the value and its first
/// kernel element. A logic leaf's instances are listed last first, which is their order in the
/// kernel (`Leaf::kstart`), so they join one run.
#[derive(Clone, Copy)]
struct Item {
    leaf: u32,
    voff: u32,
    k: u32,
    /// The leaf's `len`, `vsize`, `kind` and `bias`, at hand.
    len: u32,
    vsize: u32,
    kind: LeafKind,
    bias: u8,
}

/// The most instances of leaves a value may have to get `Bound::items`; larger ones go leaf by
/// leaf.
const MAX_ITEMS: usize = 4096;

/// Items `first..first + n` of a bound plan. In `reads`: logic leaves of one signal inside plane
/// word `word` of it (`NO_WORD`: one leaf read on its own). In `runs`: logic leaves of one signal
/// next to each other, at most 64 elements (`word` the element count; `NO_WORD`: one leaf driven
/// on its own). Kept in registers while a group is read or driven: no per-leaf signal lookup and
/// no plane block built in memory (#790).
#[derive(Clone, Copy)]
struct Grp {
    first: u32,
    n: u32,
    word: u32,
    sig: yavs_kernel::SignalId,
}

const NO_WORD: u32 = u32::MAX;

/// Whether leaf `l` of signal `s` is a logic leaf of 1 to 64 elements (`s.ty` is the leaf's own
/// type; `s.off` where it starts in its kernel signal).
fn logic64(l: &Leaf, s: &InstSig) -> bool {
    l.logic()
        && (1..=64).contains(&l.len)
        && l.len <= s.ty.len
        && s.off.checked_add(l.len).is_some()
}

/// Items, `reads` and `runs` of a `Bound`.
type Grouped = (Box<[Item]>, Box<[Grp]>, Box<[Grp]>);

/// The items of a whole value and their `reads` and `runs` groups (`Bound`); none for a value of
/// more than `MAX_ITEMS` leaf instances.
fn groups(plan: &Plan, sigs: &[InstSig]) -> Grouped {
    let total: u64 = plan.leaves.iter().map(|l| u64::from(l.insts)).sum();
    if total as usize > MAX_ITEMS {
        return (Box::new([]), Box::new([]), Box::new([]));
    }
    let mut items = Vec::with_capacity(total as usize);
    for (i, (l, s)) in plan.leaves.iter().zip(sigs).enumerate() {
        let n = l.insts as usize;
        for x in 0..n {
            let j = if l.logic() { n - 1 - x } else { x };
            let k = u64::from(s.off) + l.kstart(j, 0, plan.count);
            let (Ok(k), Ok(voff)) = (u32::try_from(k), u32::try_from(l.off(j))) else {
                return (Box::new([]), Box::new([]), Box::new([]));
            };
            items.push(Item {
                leaf: i as u32,
                voff,
                k,
                len: l.len,
                vsize: l.vsize,
                kind: l.kind,
                bias: l.bias,
            });
        }
    }
    let (mut reads, mut runs) = (Vec::<Grp>::new(), Vec::<Grp>::new());
    for (i, t) in items.iter().enumerate() {
        let i = i as u32;
        let (l, s) = (&plan.leaves[t.leaf as usize], &sigs[t.leaf as usize]);
        let ok = logic64(l, s) && t.k.checked_add(l.len).is_some();
        let (w, w_end) = (t.k / 64, t.k.saturating_add(l.len.max(1) - 1) / 64);
        match reads.last_mut() {
            Some(g) if ok && w == w_end && g.word == w && g.sig == s.signal => g.n += 1,
            _ => reads.push(Grp {
                first: i,
                n: 1,
                word: if ok && w == w_end { w } else { NO_WORD },
                sig: s.signal,
            }),
        }
        match runs.last_mut() {
            Some(g)
                if ok && g.word != NO_WORD && g.word + l.len <= 64 && {
                    g.sig == s.signal && items[g.first as usize].k + g.word == t.k
                } =>
            {
                g.n += 1;
                g.word += l.len;
            }
            _ => runs.push(Grp {
                first: i,
                n: 1,
                word: if ok { l.len } else { NO_WORD },
                sig: s.signal,
            }),
        }
    }
    (items.into(), reads.into(), runs.into())
}

/// The planes of a logic leaf of `kind`, `len` (1 to 64) elements and `bias` whose value is `v`,
/// in the low bits.
#[inline(always)]
fn leaf_planes(kind: LeafKind, len: u32, bias: u8, v: &[u8]) -> [u64; 4] {
    match kind {
        LeafKind::LogicScalar => {
            let code = v.first().map_or(0, |b| b.wrapping_add(bias));
            std::array::from_fn(|i| u64::from(code >> i & 1))
        }
        _ => {
            let w = v.as_chunks::<8>().0;
            let mask = if len == 64 { u64::MAX } else { (1 << len) - 1 };
            [0, 1, 2, 3].map(|i| w.get(i).map_or(0, |c| u64::from_le_bytes(*c)) & mask)
        }
    }
}

/// The bytes of leaf table `tab`.
fn table(prog: &Program, tab: ConstId) -> R<&[u8]> {
    match prog.consts.get(tab) {
        Some(ConstData::Bytes(b)) => Ok(b),
        _ => Err(cold(|| ice("a leaf table that is not a byte constant"))),
    }
}

/// The low `o.len()` bytes of `v` into `o` (no `memcpy` call for the usual sizes).
#[inline(always)]
fn put_le(o: &mut [u8], v: u64) {
    match o.len() {
        1 => o[0] = v as u8,
        4 => o.copy_from_slice(&(v as u32).to_le_bytes()),
        8 => o.copy_from_slice(&v.to_le_bytes()),
        n => {
            let n = n.min(8);
            o[..n].copy_from_slice(&v.to_le_bytes()[..n]);
        }
    }
}

/// The part of the signal a gather or scatter works on: the first element and the number of
/// elements of the outermost array of the whole signal (the value's own count, for a whole one).
fn part_of(m: &Machine, plan: &Plan, part: Option<Part>) -> R<(u64, u64)> {
    let (first, outer) = match part {
        Some(q) => (m.r(q.first), m.r(q.outer)),
        None => (0, plan.count),
    };
    if first.checked_add(plan.count).is_none_or(|e| e > outer) {
        return Err(cold(|| {
            ice(format!(
                "elements {first} + {} of a record signal of {outer}",
                plan.count
            ))
        }));
    }
    Ok((first, outer))
}

/// Instance of leaf `l` of signal `s` at element `k`, read into its place `o` in the value.
#[inline(always)]
fn read_leaf(c: &Ctx<'_>, s: &InstSig, l: &Leaf, k: u32, o: &mut [u8]) -> R<()> {
    match l.kind {
        LeafKind::LogicScalar => {
            let v = u64::from(c.elem_code(s.signal, k)?.wrapping_sub(l.bias));
            put_le(o, v);
        }
        LeafKind::Scalar => {
            let v = match c.elem_bytes(s.signal, k) {
                Some(b) => cell(b),
                None => cell(c.value(s.signal.slice(k, 1))?),
            };
            put_le(o, v);
        }
        LeafKind::LogicVec => {
            // A vector inside one plane block (every vector leaf of a record pack, #686): its
            // bits straight from the signal's planes.
            if l.len > 0
                && let Some((planes, n)) = c.planes(s.signal)
                && k.checked_add(l.len).is_some_and(|e| e <= n)
                && (k % 64) + l.len <= 64
            {
                let (blk, at) = (&planes[(k / 64) as usize], k % 64);
                let mask = if l.len == 64 {
                    u64::MAX
                } else {
                    (1 << l.len) - 1
                };
                for (i, w) in o.as_chunks_mut::<8>().0.iter_mut().take(4).enumerate() {
                    *w = (blk[i] >> at & mask).to_le_bytes();
                }
            } else {
                c.read_into(s.signal.slice(k, l.len), o)?;
            }
        }
        LeafKind::Bytes => c.read_into(s.signal.slice(k, l.len), o)?,
    }
    Ok(())
}

/// Logic leaves next to each other in one kernel driver (a pack, #686) collected into one plane
/// block: the driver, the elements and the planes so far.
struct Run {
    id: yavs_kernel::DriverId,
    r: Range,
    p: [u64; 4],
}

/// The delay of a scatter, whether it is none (`now`), and the delay mode.
type When = (yavs_kernel::Time, bool, yavs_kernel::DelayMode);

/// Assign the run `run` makes, if there is one.
#[inline(always)]
fn flush_run(c: &mut Ctx<'_>, run: &mut Option<Run>, when: When) -> R<()> {
    let Some(Run { id, r, p }) = run.take() else {
        return Ok(());
    };
    flush_planes(c, id, r, p, when)
}

/// Assign planes `p` to elements `r` (at most 64) of driver `id`.
#[inline(always)]
fn flush_planes(
    c: &mut Ctx<'_>,
    id: yavs_kernel::DriverId,
    r: Range,
    p: [u64; 4],
    (t, now, mode): When,
) -> R<()> {
    if r.len == 1 {
        let code = (0..4).fold(0u8, |a, i| a | ((p[i] & 1) as u8) << i);
        if now && c.assign_code_now(id, r, code) {
            return Ok(());
        }
        c.assign_code(id, Some(r), t, code, mode)?;
        return Ok(());
    }
    if now && c.assign_planes_now(id, Some(r), p) {
        return Ok(());
    }
    let mut b = [0u8; 32];
    for (k, w) in p.iter().enumerate() {
        b[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    c.assign_blocks(id, Some(r), t, &b, mode)?;
    Ok(())
}

/// Instance of leaf `l`, whose value is `v`, driven onto elements `r` of its signal through driver
/// `d` after `when`; a logic leaf joins the run of the leaves before it when it follows them in
/// the same driver.
#[inline(always)]
fn write_leaf(
    c: &mut Ctx<'_>,
    d: yavs_kernel::DriverId,
    l: &Leaf,
    r: Range,
    v: &[u8],
    run: &mut Option<Run>,
    when: When,
) -> R<()> {
    let (t, now, mode) = when;
    if matches!(l.kind, LeafKind::LogicScalar | LeafKind::LogicVec) && (1..=64).contains(&l.len) {
        let joins = run.as_ref().is_some_and(|q| {
            q.id == d && q.r.offset + q.r.len == r.offset && q.r.len + r.len <= 64
        });
        if !joins {
            flush_run(c, run, when)?;
            *run = Some(Run {
                id: d,
                r: Range::new(r.offset, 0),
                p: [0; 4],
            });
        }
        let Some(q) = run.as_mut() else {
            return Err(cold(|| ice("a run that was just made")));
        };
        let at = q.r.len;
        let bits = leaf_planes(l.kind, l.len, l.bias, v);
        for (p, b) in q.p.iter_mut().zip(bits) {
            *p |= b << at;
        }
        q.r.len += r.len;
        return Ok(());
    }
    flush_run(c, run, when)?;
    match l.kind {
        LeafKind::LogicScalar | LeafKind::LogicVec => {
            c.assign_blocks(d, Some(r), t, v, mode)?;
        }
        LeafKind::Scalar | LeafKind::Bytes => {
            let v = v.get(..l.len as usize * l.ew as usize).unwrap_or(v);
            if !(now && l.len == 1 && c.assign_bytes_now(d, r, v)) {
                c.assign_bytes(d, Some(r), t, v, mode)?;
            }
        }
    }
    Ok(())
}

impl Machine {
    /// The plan of table `tab` for the value whose descriptor is at `desc`, out of this machine's
    /// list; give it back with [`Machine::put_plan`].
    #[inline(always)]
    fn take_plan(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Box<Bound>)> {
        for i in 0..self.plans.len() {
            let m = &self.plans[i];
            if m.tab != tab.0 || m.bound.is_none() {
                continue;
            }
            let hit = m.ds == 0 || m.at == desc || {
                let same = *self.rd(desc, m.ds)? == *m.desc;
                if same && split(desc).0 == CONST {
                    self.plans[i].at = desc;
                }
                same
            };
            if hit && let Some(b) = self.plans[i].bound.take() {
                return Ok((i, b));
            }
        }
        self.new_bound(prog, tab, desc)
    }

    /// Give back what [`Machine::take_plan`] took.
    #[inline(always)]
    fn put_plan(&mut self, at: usize, b: Box<Bound>) {
        if let Some(m) = self.plans.get_mut(at) {
            m.bound = Some(b);
        }
    }

    /// [`Machine::take_plan`] for a table and descriptor this machine has not met (or lost the plan
    /// of to an error): the plan from the runtime, bound to this machine's signals.
    #[cold]
    #[inline(never)]
    fn new_bound(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Box<Bound>)> {
        let (plan, ds) = self.leaf_plan(prog, tab, desc)?;
        let d = self.rd(desc, ds)?.to_vec();
        let mut sigs = Vec::with_capacity(plan.leaves.len());
        for l in &plan.leaves {
            let s = *self
                .inst
                .sigs
                .get(l.sig as usize)
                .ok_or_else(|| ice(format!("no instance signal {}", l.sig)))?;
            let ok = match l.kind {
                LeafKind::LogicScalar | LeafKind::LogicVec => s.ty.elem == Elem::Logic,
                LeafKind::Scalar => matches!(s.ty.elem, Elem::Bytes(es) if es <= 8),
                LeafKind::Bytes => matches!(s.ty.elem, Elem::Bytes(_)),
            };
            if !ok {
                return Err(ice("a leaf table that does not match its signal"));
            }
            sigs.push(s);
        }
        let at = match self
            .plans
            .iter()
            .position(|m| m.tab == tab.0 && *m.desc == *d)
        {
            Some(at) => at,
            None => {
                self.plans.push(MPlan {
                    tab: tab.0,
                    ds,
                    at: NO_ADDR,
                    desc: d.into(),
                    bound: None,
                });
                self.plans.len() - 1
            }
        };
        Ok((at, {
            let (items, reads, runs) = groups(&plan, &sigs);
            Box::new(Bound {
                plan,
                sigs: sigs.into(),
                items,
                reads,
                runs,
            })
        }))
    }

    /// The runtime's plan of table `tab` for the value whose descriptor is at `desc`, and the
    /// descriptor size of the table's type.
    #[inline(never)]
    fn leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<(Arc<Plan>, u64)> {
        let key = tab.0;
        if let Ok(m) = self.rt.plans.0.read()
            && let Some((ds, v)) = m.get(&key)
        {
            let d = self.rd(desc, *ds)?;
            if let Some((_, p)) = v.iter().find(|(k, _)| **k == *d) {
                return Ok((Arc::clone(p), *ds));
            }
        }
        self.new_leaf_plan(prog, tab, desc)
    }

    /// [`Machine::leaf_plan`] for a table and descriptor not seen before.
    #[cold]
    #[inline(never)]
    fn new_leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<(Arc<Plan>, u64)> {
        let bytes = table(prog, tab)?;
        let Some((ty, _)) = gather::entries(bytes) else {
            return Err(cold(|| ice("a leaf table that is not one")));
        };
        let ds = Ty::desc_size(&prog.types, ty)
            .ok_or_else(|| cold(|| ice("a leaf table of a type without a descriptor size")))?;
        let d = self.rd(desc, ds)?;
        let plan = match gather::plan(&prog.types, &prog.consts, bytes, d) {
            Ok(p) => Arc::new(p),
            Err(PlanError::Layout(yavs_ir::LayoutError::TooLarge)) => {
                return cold(|| rt_err("object size exceeds 64 bits"));
            }
            Err(e) => return Err(cold(|| ice(format!("leaf table: {e:?}")))),
        };
        if let Ok(mut m) = self.rt.plans.0.write() {
            m.entry(tab.0)
                .or_insert_with(|| (ds, Vec::new()))
                .1
                .push((d.into(), Arc::clone(&plan)));
        }
        Ok((plan, ds))
    }

    /// `Inst::SigGather`.
    #[inline(never)]
    pub(crate) fn sig_gather(
        &mut self,
        prog: &Program,
        dst: Reg,
        desc: Reg,
        tab: ConstId,
        part: Option<Part>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let (at, b) = self.take_plan(prog, tab, self.r(desc))?;
        // The plan goes back on both arms: an `R` kept across `put_plan` is reloaded through
        // memory, a stall of its own.
        match self.gather_leaves(&b, dst, part, ctx) {
            Ok(()) => {
                self.put_plan(at, b);
                Ok(())
            }
            Err(e) => {
                self.put_plan(at, b);
                Err(e)
            }
        }
    }

    fn gather_leaves(
        &mut self,
        b: &Bound,
        dst: Reg,
        part: Option<Part>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = &*b.plan;
        let (first, outer) = part_of(self, plan, part)?;
        let Some(c) = ctx.as_deref_mut() else {
            return cold(|| rt_err("signal or wait operation during elaboration"));
        };
        let base = self.r(dst);
        let out = self.wr(base, plan.size)?;
        if part.is_none() && !b.items.is_empty() {
            for g in &*b.reads {
                let ts = &b.items[g.first as usize..][..g.n as usize];
                // One plane word of the signal for all items of the group.
                let blk = match g.word {
                    NO_WORD => None,
                    w => c
                        .planes(g.sig)
                        .and_then(|(p, _)| p.get(w as usize).copied()),
                };
                for t in ts {
                    let at = t.voff as usize;
                    let o = out
                        .get_mut(at..at + t.vsize as usize)
                        .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                    let Some(blk) = blk else {
                        let (l, s) = (&plan.leaves[t.leaf as usize], &b.sigs[t.leaf as usize]);
                        read_leaf(c, s, l, t.k, o)?;
                        continue;
                    };
                    let sh = t.k % 64;
                    if t.kind == LeafKind::LogicScalar {
                        let code = (0..4).fold(0u8, |a, i| a | ((blk[i] >> sh & 1) as u8) << i);
                        put_le(o, u64::from(code.wrapping_sub(t.bias)));
                    } else {
                        let mask = if t.len == 64 {
                            u64::MAX
                        } else {
                            (1 << t.len) - 1
                        };
                        for (i, w) in o.as_chunks_mut::<8>().0.iter_mut().take(4).enumerate() {
                            *w = (blk[i] >> sh & mask).to_le_bytes();
                        }
                    }
                }
            }
            return Ok(());
        }
        for (l, s) in plan.leaves.iter().zip(&*b.sigs) {
            for j in 0..l.insts as usize {
                let at = l.off(j);
                let o = out
                    .get_mut(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                read_leaf(c, s, l, s.off + l.kstart(j, first, outer) as u32, o)?;
            }
        }
        Ok(())
    }

    /// `Inst::SigScatter`.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn sig_scatter(
        &mut self,
        prog: &Program,
        f: &Func,
        driver: u32,
        src: Reg,
        desc: Reg,
        tab: ConstId,
        ops: Regs,
        mode: DriveMode,
        has: u8,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let Some(&[after, ref rest @ ..]) = f.regs(ops) else {
            return Err(cold(|| ice("a scatter without its operands")));
        };
        let (reject, part) = match (has & SCATTER_REJECT != 0, has & SCATTER_PART != 0, rest) {
            (false, false, []) => (None, None),
            (true, false, [r]) => (Some(*r), None),
            (false, true, [a, b]) => (
                None,
                Some(Part {
                    first: *a,
                    outer: *b,
                }),
            ),
            (true, true, [r, a, b]) => (
                Some(*r),
                Some(Part {
                    first: *a,
                    outer: *b,
                }),
            ),
            _ => return Err(cold(|| ice("a scatter with the wrong operands"))),
        };
        let (at, b) = self.take_plan(prog, tab, self.r(desc))?;
        match self.scatter_leaves(&b, driver, src, part, after, mode, reject, ctx) {
            Ok(()) => {
                self.put_plan(at, b);
                Ok(())
            }
            Err(e) => {
                self.put_plan(at, b);
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scatter_leaves(
        &mut self,
        b: &Bound,
        driver: u32,
        src: Reg,
        part: Option<Part>,
        after: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = &*b.plan;
        let (first, outer) = part_of(self, plan, part)?;
        let t = to_time(self.ri(after))?;
        let now = t == yavs_kernel::Time::ZERO && reject.is_none();
        let mode = self.delay_mode(mode, reject)?;
        let Some(c) = ctx.as_deref_mut() else {
            return cold(|| rt_err("signal or wait operation during elaboration"));
        };
        let data = self.rd(self.r(src), plan.size)?;
        let ds = self
            .drivers
            .get(driver as usize..driver as usize + plan.leaves.len())
            .ok_or_else(|| cold(|| ice(format!("no driver slots from {driver}"))))?;
        let when = (t, now, mode);
        let mut run = None;
        if part.is_none() && !b.items.is_empty() {
            for g in &*b.runs {
                let ts = &b.items[g.first as usize..][..g.n as usize];
                let id = ds[ts[0].leaf as usize].id;
                // One run of one driver: its planes built in registers, one assignment.
                if g.word != NO_WORD && ts.iter().all(|t| ds[t.leaf as usize].id == id) {
                    flush_run(c, &mut run, when)?;
                    let (mut p, mut at) = ([0u64; 4], 0);
                    for t in ts {
                        let o = t.voff as usize;
                        let v = data
                            .get(o..o + t.vsize as usize)
                            .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                        for (p, b) in p.iter_mut().zip(leaf_planes(t.kind, t.len, t.bias, v)) {
                            *p |= b << at;
                        }
                        at += t.len;
                    }
                    flush_planes(c, id, Range::new(ts[0].k, at), p, when)?;
                    continue;
                }
                for t in ts {
                    let l = &plan.leaves[t.leaf as usize];
                    let at = t.voff as usize;
                    let v = data
                        .get(at..at + l.vsize as usize)
                        .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                    let d = ds[t.leaf as usize].id;
                    write_leaf(c, d, l, Range::new(t.k, l.len), v, &mut run, when)?;
                }
            }
            return flush_run(c, &mut run, when);
        }
        for ((l, s), d) in plan.leaves.iter().zip(&*b.sigs).zip(ds) {
            let n = l.insts as usize;
            for i in 0..n {
                // A logic leaf's instances are mirrored in the kernel (`Leaf::kstart`): taken
                // last first, they follow each other and join one run of the driver.
                let j = if l.logic() { n - 1 - i } else { i };
                let at = l.off(j);
                let v = data
                    .get(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                let r = Range::new(s.off + l.kstart(j, first, outer) as u32, l.len);
                write_leaf(c, d.id, l, r, v, &mut run, when)?;
            }
        }
        flush_run(c, &mut run, when)
    }
}
