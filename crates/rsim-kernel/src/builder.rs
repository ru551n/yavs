//! Elaboration-free model construction.

use std::sync::Mutex;

use crate::error::{ErrorKind, SimError};
use crate::kernel::{Config, ProcMeta, SimControl, Slot};
use crate::process::{DriverInfo, ProcId, ProcState, Process, check_ref};
use crate::signal::{
    DriverId, DriverSlot, Range, Resolver, SigRef, Signal, SignalId, SignalType, Waveform,
};

/// Process attributes fixed at elaboration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcFlags {
    /// `postponed process`.
    pub postponed: bool,
    /// Touches non-protected shared variables: never runs concurrently.
    pub serial: bool,
}

struct ProcDecl {
    name: String,
    flags: ProcFlags,
    process: Box<dyn Process>,
    drivers: Vec<DriverId>,
}

/// Builds a model: signals, drivers and processes, in elaboration order.
/// Errors are deferred to [`Builder::build`].
#[derive(Default)]
pub struct Builder {
    signals: Vec<Signal>,
    drivers: Vec<(SignalId, Range, Option<ProcId>)>,
    procs: Vec<ProcDecl>,
    error: Option<SimError>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    fn fail(&mut self, msg: String) {
        self.error
            .get_or_insert(SimError::new(ErrorKind::Usage(msg)));
    }

    /// An unresolved signal with initial value `init`.
    pub fn signal(&mut self, name: &str, ty: SignalType, init: &[u8]) -> SignalId {
        self.add_signal(name, ty, init, None)
    }

    /// A resolved signal; `resolver` runs over every driven segment.
    pub fn resolved_signal(
        &mut self,
        name: &str,
        ty: SignalType,
        init: &[u8],
        resolver: Resolver,
    ) -> SignalId {
        self.add_signal(name, ty, init, Some(resolver))
    }

    fn add_signal(
        &mut self,
        name: &str,
        ty: SignalType,
        init: &[u8],
        res: Option<Resolver>,
    ) -> SignalId {
        let size = ty.elem_size.checked_mul(ty.len).map(|n| n as usize);
        if ty.elem_size == 0 || ty.len == 0 || size != Some(init.len()) {
            self.fail(format!(
                "signal {name}: initial value of {} bytes does not match type {ty:?}",
                init.len()
            ));
        }
        self.signals
            .push(Signal::new(name.to_string(), ty, init, res));
        SignalId(self.signals.len() as u32 - 1)
    }

    /// The implicit `s'transaction` signal: one byte, toggling 0/1 in every
    /// cycle in which `s` is active.
    pub fn transaction_signal(&mut self, s: SignalId) -> SignalId {
        let name = match check_ref(&self.signals, s.into()) {
            Ok((sig, _)) => format!("{}'transaction", sig.name),
            Err(k) => {
                self.error.get_or_insert(SimError::new(k));
                format!("<invalid {}>'transaction", s.0)
            }
        };
        let t = self.signal(
            &name,
            SignalType {
                elem_size: 1,
                len: 1,
            },
            &[0],
        );
        if let Some(sig) = self.signals.get_mut(s.0 as usize) {
            sig.transaction = Some(t);
        }
        t
    }

    /// A driver for `target`, to be owned by the next process that lists it.
    pub fn driver(&mut self, target: impl Into<SigRef>) -> DriverId {
        let target = target.into();
        let range = match check_ref(&self.signals, target) {
            Ok((_, range)) => range,
            Err(k) => {
                self.error.get_or_insert(SimError::new(k));
                Range::new(0, 0)
            }
        };
        self.drivers.push((target.signal, range, None));
        DriverId(self.drivers.len() as u32 - 1)
    }

    /// A process owning `drivers`.
    pub fn process(
        &mut self,
        name: &str,
        flags: ProcFlags,
        drivers: &[DriverId],
        process: impl Process + 'static,
    ) -> ProcId {
        let pid = ProcId(self.procs.len() as u32);
        for d in drivers {
            match self.drivers.get_mut(d.0 as usize).map(|d| &mut d.2) {
                None => self.fail(format!("process {name}: no driver {}", d.0)),
                Some(Some(_)) => self.fail(format!("process {name}: driver {} already owned", d.0)),
                Some(owner) => *owner = Some(pid),
            }
        }
        self.procs.push(ProcDecl {
            name: name.to_string(),
            flags,
            process: Box::new(process),
            drivers: drivers.to_vec(),
        });
        pid
    }

    pub fn build(mut self, cfg: Config) -> Result<SimControl, SimError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let mut infos = Vec::with_capacity(self.drivers.len());
        for (i, &(sig, range, owner)) in self.drivers.iter().enumerate() {
            let usage = |m: String| SimError::new(ErrorKind::Usage(m));
            let owner = owner.ok_or_else(|| usage(format!("driver {i} has no process")))?;
            let local = self.procs[owner.0 as usize]
                .drivers
                .iter()
                .position(|d| d.0 == i as u32);
            let local = local.ok_or_else(|| {
                SimError::new(ErrorKind::Internal(format!("driver {i} not in its owner")))
            })?;
            let s = &mut self.signals[sig.0 as usize];
            if s.resolver.is_none() && s.drivers.iter().any(|d| d.range.overlaps(range)) {
                return Err(usage(format!(
                    "unresolved signal {} has several drivers",
                    s.name
                )));
            }
            let value = s.cur[s.bytes(range)].to_vec();
            infos.push(DriverInfo {
                owner,
                local: local as u32,
                signal: sig,
                range,
                slot: s.drivers.len() as u32,
            });
            s.drivers.push(DriverSlot { range, value });
        }
        for s in &mut self.signals {
            s.build_segments();
            s.compute_driving(s.full()).map_err(SimError::new)?;
            s.cur.clone_from(&s.driving);
            s.last_value.clone_from(&s.driving);
        }
        let mut slots = Vec::new();
        let mut procs = Vec::new();
        for p in self.procs {
            let st = ProcState {
                waveforms: p.drivers.iter().map(|_| Waveform::default()).collect(),
                ..Default::default()
            };
            slots.push(Mutex::new(Slot {
                process: p.process,
                st,
                result: Ok(()),
            }));
            procs.push(ProcMeta {
                name: p.name,
                flags: p.flags,
                generation: 0,
                ready: false,
            });
        }
        SimControl::new(cfg, self.signals, infos, slots, procs)
    }
}
