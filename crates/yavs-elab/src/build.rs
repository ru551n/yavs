//! Building the kernel model from an [`Elaborated`] design. `build_with` can be called any
//! number of times on the same design (in-process restart, rule 4).

use yavs_kernel::{
    Builder, Config, DriverId, PortConv, ProcFlags, Process, Resolution, SimControl, SimError,
};

use crate::hier::{Case, ImplicitKind, LeafRes};
use crate::{Elaborated, ProcInst};

struct Boxed(Box<dyn Process>);

impl Process for Boxed {
    fn run(&mut self, ctx: &mut yavs_kernel::Ctx<'_>) -> Result<(), SimError> {
        self.0.run(ctx)
    }
    fn serial(&self) -> bool {
        self.0.serial()
    }
}

fn usage(m: String) -> SimError {
    SimError::new(yavs_kernel::ErrorKind::Usage(m))
}

impl Elaborated {
    /// Kernel signal names: the leaf's object path, plus record fields.
    pub fn leaf_name(&self, l: crate::LeafId) -> String {
        let leaf = self.nets.leaf(l);
        let o = self.nets.obj(leaf.obj);
        let mut n = format!(
            "{}{}",
            self.hier.path_name(o.scope, Case::Original),
            o.name.text
        );
        let mut shape = &o.shape;
        for &f in &leaf.path {
            loop {
                match shape {
                    crate::Shape::Array { elem, .. } => shape = elem,
                    crate::Shape::Record { fields, .. } => {
                        let (name, s) = &fields[f as usize];
                        n.push('.');
                        n.push_str(name.text.as_str());
                        shape = s;
                        break;
                    }
                    crate::Shape::Scalar { .. } => break,
                }
            }
        }
        n
    }

    /// The kernel model. `make` creates the process object of each process instance, given
    /// its kernel drivers (in `ProcInst::drivers` order).
    pub fn build_with(
        &self,
        cfg: Config,
        make: impl FnMut(&ProcInst, &[DriverId]) -> Result<Box<dyn Process>, SimError>,
    ) -> Result<SimControl, SimError> {
        self.build_res(cfg, make, |_, _| {
            Err(usage(
                "user resolution functions need the lowered program (Elaborated::build)".into(),
            ))
        })
    }

    /// [`Elaborated::build_with`], with `res` giving the kernel resolver of a user
    /// resolution function.
    pub(crate) fn build_res(
        &self,
        cfg: Config,
        mut make: impl FnMut(&ProcInst, &[DriverId]) -> Result<Box<dyn Process>, SimError>,
        res: impl Fn(yavs_sem::DeclId, crate::LeafId) -> Result<yavs_kernel::Resolver, SimError>,
    ) -> Result<SimControl, SimError> {
        let mut b = Builder::new();
        for (i, leaf) in self.nets.leaves.iter().enumerate() {
            let name = self.leaf_name(crate::LeafId(i as u32));
            let id = match (&leaf.resolution, leaf.implicit) {
                (_, Some((k, src, r))) => {
                    let src = yavs_kernel::SignalId(src.0).slice(r.offset, r.len);
                    match k {
                        ImplicitKind::Stable(t) => b.stable_signal(src, t),
                        ImplicitKind::Quiet(t) => b.quiet_signal(src, t),
                        ImplicitKind::Delayed(t) => b.delayed_signal(src, t),
                        ImplicitKind::Transaction => b.transaction_signal(src),
                    }
                }
                (None, None) => b.signal(&name, leaf.ty, &leaf.init),
                (Some(LeafRes::StdLogic), None) => {
                    b.resolved_signal(&name, leaf.ty, &leaf.init, Resolution::StdLogic)
                }
                (Some(LeafRes::Func(_)), None)
                    if matches!(leaf.ty.elem, yavs_kernel::Elem::Logic) =>
                {
                    return Err(usage(format!(
                        "signal {name}: user resolution of std_ulogic, bit and boolean leaves \
                         is not implemented yet (M3)"
                    )));
                }
                (Some(LeafRes::Func(d)), None) => b.resolved_signal(
                    &name,
                    leaf.ty,
                    &leaf.init,
                    Resolution::Func(res(*d, crate::LeafId(i as u32))?),
                ),
            };
            debug_assert_eq!(id.0 as usize, i, "kernel SignalId equals LeafId");
            if let Some(g) = leaf.guard {
                b.guard(id, g, yavs_kernel::Time::ZERO);
            }
        }
        for e in &self.nets.ports {
            let sig = |(l, r): (crate::LeafId, yavs_kernel::Range)| {
                yavs_kernel::SignalId(l.0).slice(r.offset, r.len)
            };
            let conv = |i: Option<u32>| -> Option<yavs_kernel::Converter> {
                let c = self.convs.get(i? as usize)?.clone();
                Some(std::sync::Arc::new(move |v: yavs_kernel::Value| {
                    c.apply(&v)
                }))
            };
            let pc = PortConv {
                to_formal: conv(e.to_formal),
                to_actual: conv(e.to_actual),
            };
            b.port(sig(e.formal), sig(e.actual), e.mode, pc);
        }
        for p in &self.procs {
            let ds: Vec<DriverId> = p
                .drivers
                .iter()
                .map(|&d| {
                    let d = &self.nets.drivers[d];
                    b.driver(yavs_kernel::SignalId(d.leaf.0).slice(d.range.offset, d.range.len))
                })
                .collect();
            let proc = make(p, &ds)?;
            let name = self.hier.path_name(p.scope, Case::Lrm);
            let name = name.strip_suffix(':').unwrap_or(&name);
            b.process(
                name,
                ProcFlags {
                    postponed: p.postponed,
                    serial: false,
                },
                &ds,
                Boxed(proc),
            );
        }
        b.build(cfg)
    }
}
