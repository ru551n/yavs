//! Building the kernel model from an [`Elaborated`] design. `build_with` can be called any
//! number of times on the same design (in-process restart, rule 4).

use yavs_kernel::{
    Builder, Config, DriverId, EdgeTrigger, PortConv, ProcFlags, Process, Resolution, SimControl,
    SimError,
};

use crate::hier::{Case, ImplicitKind, LeafRes};
use crate::{Elaborated, ProcInst};

fn usage(m: String) -> SimError {
    SimError::new(yavs_kernel::ErrorKind::Usage(m))
}

impl Elaborated {
    /// Kernel signal names: the leaf's object path, plus record fields.
    pub fn leaf_name(&self, l: crate::LeafId) -> String {
        let leaf = self.nets.leaf(l);
        let o = self.nets.obj(leaf.obj);
        let mut n = self.hier.path_name(o.scope, Case::Original);
        n.push_str(o.name.text.as_str());
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
                    let id = match k {
                        ImplicitKind::Stable(t) => b.stable_signal(src, t),
                        ImplicitKind::Quiet(t) => b.quiet_signal(src, t),
                        ImplicitKind::Delayed(t) => b.delayed_signal(src, t),
                        ImplicitKind::Transaction => b.transaction_signal(src),
                    };
                    for &(l, r) in &leaf.implicit_more {
                        b.implicit_source(id, yavs_kernel::SignalId(l.0).slice(r.offset, r.len));
                    }
                    id
                }
                (None, None) => b.signal(&name, leaf.ty, &leaf.init),
                (Some(LeafRes::StdLogic), None) => {
                    b.resolved_signal(&name, leaf.ty, &leaf.init, Resolution::StdLogic)
                }
                // Joined into one resolved composite below.
                (Some(LeafRes::Whole(_)), None) => b.signal(&name, leaf.ty, &leaf.init),
                (Some(LeafRes::Func(d)), None) => b.resolved_signal(
                    &name,
                    leaf.ty,
                    &leaf.init,
                    Resolution::Func(res(*d, crate::LeafId(i as u32))?),
                ),
            };
            debug_assert_eq!(id.0 as usize, i, "kernel SignalId equals LeafId");
            if let Some(g) = leaf.guard {
                b.guard(id, g, leaf.disconnect);
            }
        }
        // Composites resolved as a whole (#193): one group per instance, one resolver each.
        for w in &self.nets.wholes {
            let Some(&(a, _)) = w.instances.first().and_then(|i| i.first()) else {
                continue;
            };
            if self.nets.leaf(a).implicit.is_some() {
                continue;
            }
            let f = res(w.func, a)?;
            for parts in &w.instances {
                let parts: Vec<_> = (parts.iter())
                    .map(|&(l, r)| yavs_kernel::SignalId(l.0).slice(r.offset, r.len))
                    .collect();
                b.resolved_composite(&parts, f.clone());
            }
        }
        let sig = |&(l, r): &(crate::LeafId, yavs_kernel::Range)| {
            yavs_kernel::SignalId(l.0).slice(r.offset, r.len)
        };
        let conv = |i: Option<u32>| -> Option<yavs_kernel::Converter> {
            let c = self.convs.get(i? as usize)?.clone();
            Some(std::sync::Arc::new(move |v: &[yavs_kernel::Value]| {
                c.apply(v)
            }))
        };
        for e in &self.nets.ports {
            let pc = PortConv {
                to_formal: conv(e.to_formal),
                to_actual: conv(e.to_actual),
            };
            b.port(sig(&e.formal), sig(&e.actual), e.mode, pc);
        }
        for e in &self.nets.conv_ports {
            let pc = PortConv {
                to_formal: conv(e.to_formal),
                to_actual: conv(e.to_actual),
            };
            let f: Vec<_> = e.formal.iter().map(sig).collect();
            let a: Vec<_> = e.actual.iter().map(sig).collect();
            b.port_parts(&f, &a, e.mode, pc);
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
            let proc_waits = proc.several_waits();
            let name = self.hier.path_name(p.scope, Case::Lrm);
            let name = name.strip_suffix(':').unwrap_or(&name);
            let pid = b.process_boxed(
                name,
                ProcFlags {
                    postponed: p.postponed,
                    serial: false,
                    several_waits: false,
                    concurrent_call: false,
                },
                &ds,
                proc,
            );
            if let Some(t) = p.trigger.filter(|t| !t.wait || !proc_waits) {
                b.edge_trigger(
                    pid,
                    EdgeTrigger {
                        signal: yavs_kernel::SignalId(t.leaf.0),
                        elem: t.elem,
                        rising: t.rising,
                        legacy: t.legacy,
                        reset: t.reset.map(|(l, e, c)| (yavs_kernel::SignalId(l.0), e, c)),
                    },
                );
            }
        }
        // The processes of foreign architectures' models: drivers only (#330).
        for p in &self.ext_procs {
            let ds: Vec<DriverId> = p.drivers.iter().map(|&d| b.driver(d)).collect();
            (p.built)(&ds);
            let idle = |_: &mut yavs_kernel::Ctx<'_>| Ok(());
            b.process(&p.name, ProcFlags::default(), &ds, idle);
        }
        b.build(cfg)
    }
}
