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

    /// The kernel name of a pack of leaves `a .. z` (#686): the path of the innermost record
    /// that holds them all.
    fn pack_name(&self, a: crate::LeafId, z: crate::LeafId) -> String {
        let (first, last) = (self.leaf_name(a), self.leaf_name(crate::LeafId(z.0 - 1)));
        let mut n = 0;
        for (i, (x, y)) in first.bytes().zip(last.bytes()).enumerate() {
            if x != y {
                break;
            }
            if x == b'.' {
                n = i;
            }
        }
        first[..n].to_string()
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
        res: impl Fn(yavs_sem::DeclId, crate::LeafId) -> Result<(yavs_kernel::Resolver, bool), SimError>,
    ) -> Result<SimControl, SimError> {
        let mut b = Builder::new();
        let nets = &self.nets;
        for (k, &(a, z)) in nets.packs.iter().enumerate() {
            if z.0 != a.0 + 1 {
                // A pack of record leaves (#686): logic, unresolved or `resolved`, no guard.
                let len = nets.signal_len(yavs_kernel::SignalId(k as u32));
                let mut init = yavs_kernel::Value::Logic(yavs_kernel::LogicVec::new(
                    len as usize,
                    yavs_logic::StdULogic::U,
                ));
                let mut resolved = false;
                for l in a.0..z.0 {
                    let leaf = nets.leaf(crate::LeafId(l));
                    let base = nets.loc[l as usize].base;
                    crate::collapse::put_init(&mut init, base, &leaf.init, leaf.ty.len);
                    resolved |= matches!(leaf.resolution, Some(LeafRes::StdLogic));
                }
                let name = self.pack_name(a, z);
                let ty = yavs_kernel::SignalType::logic(len);
                let id = match resolved {
                    true => b.resolved_signal(&name, ty, &init, Resolution::StdLogic),
                    false => b.signal(&name, ty, &init),
                };
                debug_assert_eq!(id.0 as usize, k, "kernel signals in pack order");
                continue;
            }
            let i = a.index();
            let leaf = nets.leaf(a);
            let name = self.leaf_name(a);
            let id = match (&leaf.resolution, leaf.implicit) {
                (_, Some((k, src, r))) => {
                    let src = nets.kref(src, r);
                    let id = match k {
                        ImplicitKind::Stable(t) => b.stable_signal(src, t),
                        ImplicitKind::Quiet(t) => b.quiet_signal(src, t),
                        ImplicitKind::Delayed(t) => b.delayed_signal(src, t),
                        ImplicitKind::Transaction => b.transaction_signal(src),
                    };
                    for &(l, r) in &leaf.implicit_more {
                        b.implicit_source(id, nets.kref(l, r));
                    }
                    id
                }
                (None, None) => b.signal(&name, leaf.ty, &leaf.init),
                (Some(LeafRes::StdLogic), None) => {
                    b.resolved_signal(&name, leaf.ty, &leaf.init, Resolution::StdLogic)
                }
                // Joined into one resolved composite below.
                (Some(LeafRes::Whole(_)), None) => b.signal(&name, leaf.ty, &leaf.init),
                (Some(LeafRes::Func(d)), None) => {
                    // The flag: the function touches shared state (never updated in parallel).
                    let (f, serial) = res(*d, crate::LeafId(i as u32))?;
                    let id = b.resolved_signal(&name, leaf.ty, &leaf.init, Resolution::Func(f));
                    if serial {
                        b.serial_resolution(id);
                    }
                    id
                }
            };
            debug_assert_eq!(id.0 as usize, k, "kernel signals in pack order");
            if self.lean.get(i) == Some(&true) {
                b.lean_events(id);
            }
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
            let (f, _) = res(w.func, a)?;
            for parts in &w.instances {
                let parts: Vec<_> = (parts.iter()).map(|&(l, r)| nets.kref(l, r)).collect();
                b.resolved_composite(&parts, f.clone());
            }
        }
        let sig = |&(l, r): &(crate::LeafId, yavs_kernel::Range)| nets.kref(l, r);
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
                    b.driver(nets.kref(d.leaf, d.range))
                })
                .collect();
            // A plain signal copy runs in the kernel when no one observes its activations
            // (coverage, a debugger, foreign callbacks: `Config::edge_triggers` is off then).
            let copy = p
                .copy
                .as_ref()
                .filter(|_| cfg.edge_triggers)
                .and_then(|src| {
                    (src.iter().zip(p.lsps.first()?))
                        .map(|(&(sl, sr), &(k, dl, dr))| {
                            let src = nets.kref(sl, sr);
                            let d = ds[p.drivers.iter().position(|&x| x == k)?];
                            Some((src, d, nets.kpart(dl, dr).1))
                        })
                        .collect::<Option<Vec<_>>>()
                });
            // So does a clock generator `x <= not x after T;`.
            let toggle = p.toggle.filter(|_| cfg.edge_triggers).and_then(|t| {
                let time = yavs_kernel::Time::from_vhdl(t.delay).ok()?;
                let k = p.lsps.first()?.first()?.0;
                let d = *ds.get(p.drivers.iter().position(|&x| x == k)?)?;
                let mode = if t.transport {
                    yavs_kernel::DelayMode::Transport
                } else {
                    yavs_kernel::DelayMode::INERTIAL
                };
                let (sig, elem) = nets.kelem(t.leaf, t.elem);
                Some(yavs_kernel::ToggleProcess::new(sig, elem, d, time, mode))
            });
            let proc: Box<dyn Process> = match (copy, toggle) {
                (Some(parts), _) => Box::new(yavs_kernel::CopyProcess::new(parts)),
                (None, Some(t)) => Box::new(t),
                (None, None) => make(p, &ds)?,
            };
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
            if let Some(reads) = &p.guard {
                let r: Vec<_> = reads.iter().map(|&(l, r)| nets.kpart(l, r)).collect();
                b.guard_process(pid, &r);
            }
            let trigger = p.trigger.filter(|t| !t.wait || !proc_waits);
            if let Some(t) = trigger {
                let (signal, elem) = nets.kelem(t.leaf, t.elem);
                b.edge_trigger(
                    pid,
                    EdgeTrigger {
                        signal,
                        elem,
                        rising: t.rising,
                        legacy: t.legacy,
                        reset: t.reset.map(|(l, e, c)| {
                            let (s, e) = nets.kelem(l, e);
                            (s, e, c)
                        }),
                    },
                );
            }
            // A pure process: the kernel may skip a wake no input had an event for (a clocked one
            // only when its edge trigger stands).
            if let Some(reads) = p
                .idle
                .as_ref()
                .filter(|_| p.trigger.is_none() || trigger.is_some())
            {
                let r: Vec<_> = reads
                    .iter()
                    .map(|r| yavs_kernel::IdleRead {
                        signal: nets.signal_of(r.leaf),
                        range: nets.kpart(r.leaf, r.range).1,
                        guard: r.guard.as_ref().map(|alts| {
                            (alts.iter())
                                .map(|conj| {
                                    (conj.iter())
                                        .map(|l| {
                                            let (signal, elem) = nets.kelem(l.leaf, l.elem);
                                            yavs_kernel::IdleLit {
                                                signal,
                                                elem,
                                                code: l.code,
                                                eq: l.eq,
                                            }
                                        })
                                        .collect()
                                })
                                .collect()
                        }),
                    })
                    .collect();
                b.idle_skip_guarded(pid, r);
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
