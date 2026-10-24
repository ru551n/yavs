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
use crate::exec::{Machine, R, cell, cold, ice, rt_err, to_time};

/// The plans of a runtime: per leaf table, the descriptor size of its value type and the plans
/// made so far with the descriptor bytes they are for. A machine copies the ones it uses
/// ([`MPlan`]); this is where the machines that meet a table and descriptor later find the plan.
#[derive(Default)]
pub(crate) struct Plans(RwLock<HashMap<u32, Known>>);

/// The descriptor size of a table's value type and the plans made for it, by descriptor bytes.
type Known = (u64, Vec<(Box<[u8]>, Arc<Plan>)>);

/// A plan of a machine's own: the table and descriptor size it is for, the descriptor bytes, and
/// the plan with the instance signal of each leaf. `bound` is taken out while an instruction uses
/// it (no lock, no shared reference count).
pub(crate) struct MPlan {
    tab: u32,
    ds: u64,
    desc: Box<[u8]>,
    bound: Option<Bound>,
}

/// A plan and the instance signal of each of its leaves.
pub(crate) struct Bound {
    plan: Arc<Plan>,
    sigs: Box<[InstSig]>,
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
fn flush_run(c: &mut Ctx<'_>, run: &mut Option<Run>, (t, now, mode): When) -> R<()> {
    let Some(Run { id, r, p }) = run.take() else {
        return Ok(());
    };
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
        let bits: [u64; 4] = match l.kind {
            LeafKind::LogicScalar => {
                let code = v[0].wrapping_add(l.bias);
                std::array::from_fn(|i| u64::from(code >> i & 1))
            }
            _ => {
                let w = v.as_chunks::<8>().0;
                let mask = if l.len == 64 {
                    u64::MAX
                } else {
                    (1 << l.len) - 1
                };
                [0, 1, 2, 3].map(|i| w.get(i).map_or(0, |c| u64::from_le_bytes(*c)) & mask)
            }
        };
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
    #[inline]
    fn take_plan(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Bound)> {
        for i in 0..self.plans.len() {
            let m = &self.plans[i];
            if m.tab == tab.0
                && m.bound.is_some()
                && (m.ds == 0 || *self.rd(desc, m.ds)? == *m.desc)
                && let Some(b) = self.plans[i].bound.take()
            {
                return Ok((i, b));
            }
        }
        self.new_bound(prog, tab, desc)
    }

    /// Give back what [`Machine::take_plan`] took.
    #[inline(always)]
    fn put_plan(&mut self, at: usize, b: Bound) {
        if let Some(m) = self.plans.get_mut(at) {
            m.bound = Some(b);
        }
    }

    /// [`Machine::take_plan`] for a table and descriptor this machine has not met (or lost the plan
    /// of to an error): the plan from the runtime, bound to this machine's signals.
    #[cold]
    #[inline(never)]
    fn new_bound(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Bound)> {
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
                    desc: d.into(),
                    bound: None,
                });
                self.plans.len() - 1
            }
        };
        Ok((
            at,
            Bound {
                plan,
                sigs: sigs.into(),
            },
        ))
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
        let r = self.gather_leaves(&b, dst, part, ctx);
        self.put_plan(at, b);
        r
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
        if plan.flat && part.is_none() {
            for (l, s) in plan.leaves.iter().zip(&*b.sigs) {
                let at = l.voff as usize;
                let o = out
                    .get_mut(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                read_leaf(c, s, l, s.off, o)?;
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
        let r = self.scatter_leaves(&b, driver, src, part, after, mode, reject, ctx);
        self.put_plan(at, b);
        r
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
        if plan.flat && part.is_none() {
            for ((l, s), d) in plan.leaves.iter().zip(&*b.sigs).zip(ds) {
                let at = l.voff as usize;
                let v = data
                    .get(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                write_leaf(c, d.id, l, Range::new(s.off, l.len), v, &mut run, when)?;
            }
            return flush_run(c, &mut run, when);
        }
        // ponytail: one assignment per instance of a leaf; the instances of a logic scalar leaf
        // are adjacent in the kernel and could go in as one plane block.
        for ((l, s), d) in plan.leaves.iter().zip(&*b.sigs).zip(ds) {
            for j in 0..l.insts as usize {
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
