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
        let known = self
            .rt
            .plans
            .0
            .read()
            .ok()
            .and_then(|m| m.get(&key).map(|e| e.0));
        let ds = match known {
            Some(ds) => ds,
            None => {
                let bytes = table(prog, tab)?;
                let Some((ty, _)) = gather::entries(bytes) else {
                    return Err(cold(|| ice("a leaf table that is not one")));
                };
                Ty::desc_size(&prog.types, ty).ok_or_else(|| {
                    cold(|| ice("a leaf table of a type without a descriptor size"))
                })?
            }
        };
        let d = self.rd(desc, ds)?;
        if let Ok(m) = self.rt.plans.0.read()
            && let Some((_, v)) = m.get(&key)
            && let Some((_, p)) = v.iter().find(|(k, _)| **k == *d)
        {
            return Ok(Arc::clone(p));
        }
        let plan = match gather::plan(&prog.types, &prog.consts, table(prog, tab)?, d) {
            Ok(p) => Arc::new(p),
            Err(PlanError::Layout(yavs_ir::LayoutError::TooLarge)) => {
                return cold(|| rt_err("object size exceeds 64 bits"));
            }
            Err(e) => return Err(cold(|| ice(format!("leaf table: {e:?}")))),
        };
        if let Ok(mut m) = self.rt.plans.0.write() {
            m.entry(key)
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
        for (k, l) in plan.leaves.iter().enumerate() {
            let s = self
                .inst
                .sigs
                .get(l.sig as usize)
                .ok_or_else(|| cold(|| ice(format!("no instance signal {}", l.sig))))?;
            let d = self
                .drivers
                .get(driver as usize + k)
                .ok_or_else(|| cold(|| ice(format!("no driver slot {}", driver as usize + k))))?;
            let (at, vs) = (l.voff as usize, l.vsize as usize);
            let b = data
                .get(at..at + vs)
                .ok_or_else(|| cold(|| ice("a leaf outside its record value")))?;
            let r = Range::new(s.off, l.len);
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
        Ok(())
    }
}
