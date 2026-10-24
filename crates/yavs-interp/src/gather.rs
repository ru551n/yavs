//! `Inst::SigGather` and `Inst::SigScatter` (#677): a whole record signal read into, or driven
//! from, a record value in one instruction, leaf by leaf in a Rust loop over the leaves of a
//! plan (`yavs_ir::gather`). Each leaf does what the `SigRead`/`SigReadScalar` and `Drive` of the
//! lowered per-leaf code did, minus the instruction dispatch, the staging buffers and the
//! register traffic. A plan is made from the leaf table and the signal's descriptor the first
//! time and kept per runtime, so a record with bounds from generics costs one comparison of its
//! descriptor.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use yavs_ir::gather::{self, LeafKind, Plan, PlanError};
use yavs_ir::{ConstData, ConstId, DriveMode, Program, Reg, Ty};
use yavs_kernel::{Ctx, Elem, Range};

use crate::exec::{Machine, R, cell, cold, ice, rt_err, to_time};

/// The plans of a runtime: per leaf table, the descriptor size of its record type and the
/// plans made so far with the descriptor bytes they are for.
#[derive(Default)]
pub(crate) struct Plans(RwLock<HashMap<u32, Known>>);

/// The descriptor size of a table's record type and the plans made for it, by descriptor bytes.
type Known = (u64, Vec<(Box<[u8]>, Arc<Plan>)>);

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

impl Machine {
    /// The plan of table `tab` for the record whose descriptor is at `desc`.
    #[inline(never)]
    fn leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<Arc<Plan>> {
        let key = tab.0;
        if let Ok(m) = self.rt.plans.0.read()
            && let Some((ds, v)) = m.get(&key)
        {
            let d = self.rd(desc, *ds)?;
            if let Some((_, p)) = v.iter().find(|(k, _)| **k == *d) {
                return Ok(Arc::clone(p));
            }
        }
        self.new_leaf_plan(prog, tab, desc)
    }

    /// [`Machine::leaf_plan`] for a table and descriptor not seen before.
    #[cold]
    #[inline(never)]
    fn new_leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<Arc<Plan>> {
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
        Ok(plan)
    }

    /// `Inst::SigGather`.
    #[inline(never)]
    pub(crate) fn sig_gather(
        &mut self,
        prog: &Program,
        dst: Reg,
        desc: Reg,
        tab: ConstId,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = self.leaf_plan(prog, tab, self.r(desc))?;
        let Some(c) = ctx.as_deref_mut() else {
            return cold(|| rt_err("signal or wait operation during elaboration"));
        };
        let inst = Arc::clone(&self.inst);
        let base = self.r(dst);
        let out = self.wr(base, plan.size)?;
        for l in &plan.leaves {
            let s = inst
                .sigs
                .get(l.sig as usize)
                .ok_or_else(|| cold(|| ice(format!("no instance signal {}", l.sig))))?;
            let (at, vs) = (l.voff as usize, l.vsize as usize);
            let o = out
                .get_mut(at..at + vs)
                .ok_or_else(|| cold(|| ice("a leaf outside its record value")))?;
            // A logic vector inside one plane block (every vector leaf of a record pack, #686):
            // its bits straight from the signal's planes.
            if l.kind == LeafKind::LogicVec
                && s.ty.elem == Elem::Logic
                && l.len > 0
                && let Some((planes, n)) = c.planes(s.signal)
                && s.off.checked_add(l.len).is_some_and(|e| e <= n)
                && (s.off % 64) + l.len <= 64
            {
                let (blk, at) = (&planes[(s.off / 64) as usize], s.off % 64);
                let mask = if l.len == 64 {
                    u64::MAX
                } else {
                    (1 << l.len) - 1
                };
                for (i, w) in o.as_chunks_mut::<8>().0.iter_mut().take(4).enumerate() {
                    *w = (blk[i] >> at & mask).to_le_bytes();
                }
                continue;
            }
            match l.kind {
                LeafKind::LogicScalar if s.ty.elem == Elem::Logic => {
                    let v = u64::from(c.elem_code(s.signal, s.off)?.wrapping_sub(l.bias));
                    put_le(o, v);
                }
                LeafKind::Scalar if matches!(s.ty.elem, Elem::Bytes(es) if es <= 8) => {
                    let v = match c.elem_bytes(s.signal, s.off) {
                        Some(b) => cell(b),
                        None => cell(c.value(s.signal.slice(s.off, 1))?),
                    };
                    put_le(o, v);
                }
                LeafKind::LogicVec | LeafKind::Bytes => {
                    c.read_into(s.signal.slice(s.off, l.len), o)?;
                }
                _ => return Err(cold(|| ice("a leaf table that does not match its signal"))),
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
        driver: u32,
        src: Reg,
        desc: Reg,
        tab: ConstId,
        after: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = self.leaf_plan(prog, tab, self.r(desc))?;
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
        // Logic leaves next to each other in one kernel driver (a pack, #686) are collected into
        // one plane block and assigned together.
        let mut run: Option<(yavs_kernel::DriverId, Range, [u64; 4])> = None;
        let flush =
            |c: &mut Ctx<'_>, run: &mut Option<(yavs_kernel::DriverId, Range, [u64; 4])>| {
                let Some((id, r, p)) = run.take() else {
                    return Ok(());
                };
                if r.len == 1 {
                    let code = (0..4).fold(0u8, |a, i| a | ((p[i] & 1) as u8) << i);
                    if now && c.assign_code_now(id, r, code) {
                        return Ok(());
                    }
                    return c.assign_code(id, Some(r), t, code, mode);
                }
                if now && c.assign_planes_now(id, Some(r), p) {
                    return Ok(());
                }
                let mut b = [0u8; 32];
                for (k, w) in p.iter().enumerate() {
                    b[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
                c.assign_blocks(id, Some(r), t, &b, mode)
            };
        for (l, d) in plan.leaves.iter().zip(ds) {
            let s = self
                .inst
                .sigs
                .get(l.sig as usize)
                .ok_or_else(|| cold(|| ice(format!("no instance signal {}", l.sig))))?;
            let (at, vs) = (l.voff as usize, l.vsize as usize);
            let b = data
                .get(at..at + vs)
                .ok_or_else(|| cold(|| ice("a leaf outside its record value")))?;
            let r = Range::new(s.off, l.len);
            if s.ty.elem == Elem::Logic
                && matches!(l.kind, LeafKind::LogicScalar | LeafKind::LogicVec)
                && (1..=64).contains(&l.len)
            {
                let joins = run.as_ref().is_some_and(|(id, q, _)| {
                    *id == d.id && q.offset + q.len == r.offset && q.len + r.len <= 64
                });
                if !joins {
                    flush(c, &mut run)?;
                    run = Some((d.id, Range::new(r.offset, 0), [0; 4]));
                }
                let (_, q, p) = run.as_mut().expect("a run was just made");
                let at = q.len;
                let bits: [u64; 4] = match l.kind {
                    LeafKind::LogicScalar => {
                        let code = b[0].wrapping_add(l.bias);
                        std::array::from_fn(|i| u64::from(code >> i & 1))
                    }
                    _ => std::array::from_fn(|i| {
                        let w = u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().expect("8"));
                        if l.len == 64 {
                            w
                        } else {
                            w & ((1 << l.len) - 1)
                        }
                    }),
                };
                for i in 0..4 {
                    p[i] |= bits[i] << at;
                }
                q.len += r.len;
                continue;
            }
            flush(c, &mut run)?;
            match l.kind {
                LeafKind::LogicScalar if s.ty.elem == Elem::Logic => {
                    let code = b[0].wrapping_add(l.bias);
                    if !(now && c.assign_code_now(d.id, r, code)) {
                        c.assign_code(d.id, Some(r), t, code, mode)?;
                    }
                }
                LeafKind::LogicVec if s.ty.elem == Elem::Logic => {
                    c.assign_blocks(d.id, Some(r), t, b, mode)?;
                }
                LeafKind::Scalar | LeafKind::Bytes if matches!(s.ty.elem, Elem::Bytes(_)) => {
                    let b = b.get(..l.len as usize * l.ew as usize).unwrap_or(b);
                    if !(now && l.len == 1 && c.assign_bytes_now(d.id, r, b)) {
                        c.assign_bytes(d.id, Some(r), t, b, mode)?;
                    }
                }
                _ => return Err(cold(|| ice("a leaf table that does not match its signal"))),
            }
        }
        flush(c, &mut run)?;
        Ok(())
    }
}
