//! Elaboration-free model construction.

use std::cmp::Reverse;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::error::{ErrorKind, SimError};
use crate::kernel::{Config, Group, Implicit, ImplicitKind, PortEdge, ProcMeta, SimControl, Slot};
use crate::process::{DriverInfo, ProcId, ProcState, Process, check_ref};
use crate::signal::{
    DriverId, DriverSlot, GuardKind, Range, Resolution, SigRef, Signal, SignalId, Waveform,
};
use crate::time::Time;
use crate::value::{Buf, Elem, SignalType, Value, ValueRef};

/// Process attributes fixed at elaboration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcFlags {
    /// `postponed process`.
    pub postponed: bool,
    /// Never runs concurrently: touches non-protected shared variables or (through M3) calls
    /// a protected type.
    pub serial: bool,
    /// Suspends at more than one point (several wait statements, or a wait in a called
    /// procedure): after initialisation it runs after the processes with one suspension point,
    /// in reverse elaboration order (#252, DESIGN.md "Parallel evaluation").
    pub several_waits: bool,
    pub concurrent_call: bool,
}

/// Port mode of a formal (LRM 6.5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortMode {
    In,
    Out,
    Inout,
    Buffer,
}

/// A conversion function or type conversion in a port association, run in the update phase.
/// It receives the value of every source part, in order, and must return one value per
/// target part, each the whole part ([`Builder::port_parts`]; one part on each side for
/// [`Builder::port`]).
pub type Converter = Arc<dyn Fn(&[Value]) -> Result<Vec<Value>, String> + Send + Sync>;

/// Conversions of one association: actual to formal (`in`, `inout`) and formal to actual
/// (`out`, `inout`, `buffer`).
#[derive(Clone, Default)]
pub struct PortConv {
    pub to_formal: Option<Converter>,
    pub to_actual: Option<Converter>,
}

struct ProcDecl {
    name: String,
    flags: ProcFlags,
    process: Box<dyn Process>,
    drivers: Vec<DriverId>,
    trigger: Option<EdgeTrigger>,
    /// The signal parts a clocked process reads, when an activation is a function of them alone
    /// (`Builder::idle_skip`).
    idle: Option<Vec<IdleRead>>,
    /// The signal parts the condition of a guarded block reads (`Builder::guard_process`).
    guard: Option<Vec<(SignalId, Range)>>,
}

/// An element of a logic signal compared with a code: `eq` it is equal to it, else it is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleLit {
    pub signal: SignalId,
    pub elem: u32,
    pub code: u8,
    pub eq: bool,
}

/// A signal part a pure process reads (`Builder::idle_skip_guarded`). With a `guard`, a
/// disjunction of conjunctions of literals, every activation that reads the part satisfies one
/// of the conjunctions: while none holds, an event on the part cannot change what the process
/// does at its next activation, because the signals that decide are read too.
#[derive(Clone, Debug)]
pub struct IdleRead {
    pub signal: SignalId,
    pub range: Range,
    pub guard: Option<Vec<Vec<IdleLit>>>,
}

/// The one condition under which an event resumes a process to any effect: the process is
/// `process (...) begin if rising_edge(s) then ... end if; end process;` (or `falling_edge`;
/// or one `wait until` of that, optionally `and`ed with more) on element `elem` of a logic
/// signal; or the same with `s'event and s = '1'`; or either behind a reset branch,
/// `if r = '1' then ... elsif <edge> then ... end if`. An event that makes neither the edge
/// nor the reset condition true would only evaluate them and suspend again, so the kernel
/// leaves the process suspended (`Config::edge_triggers`). Edges as the IEEE 1164 functions
/// define them: `'event`, and the value and `'last_value` through `To_X01`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeTrigger {
    pub signal: SignalId,
    pub elem: u32,
    pub rising: bool,
    /// The condition is `s'event and s = '1'` (`'0'` when falling) instead of the IEEE 1164
    /// function: an event ending on that value, from any value.
    pub legacy: bool,
    /// An asynchronous reset: `if s = c then ... elsif <edge> then ... end if`. The process
    /// also runs whenever element `.1` of signal `.0` holds the `std_ulogic` code `.2`.
    pub reset: Option<(SignalId, u32, u8)>,
}

/// Builds a model: signals, drivers, ports, implicit signals and processes, in elaboration
/// order. Errors are deferred to [`Builder::build`].
#[derive(Default)]
pub struct Builder {
    signals: Vec<Signal>,
    drivers: Vec<(SignalId, Range, Option<ProcId>)>,
    procs: Vec<ProcDecl>,
    ports: Vec<PortEdge>,
    implicits: Vec<Implicit>,
    groups: Vec<Group>,
    error: Option<SimError>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    fn fail(&mut self, kind: ErrorKind) {
        self.error.get_or_insert(SimError::new(kind));
    }

    fn usage(&mut self, msg: String) {
        self.fail(ErrorKind::Usage(msg));
    }

    /// A checked reference, recording an error (and returning `None`) if it is invalid.
    fn chk(&mut self, r: SigRef) -> Option<(SignalId, Range)> {
        match check_ref(&self.signals, r) {
            Ok((_, range)) => Some((r.signal, range)),
            Err(k) => {
                self.fail(k);
                None
            }
        }
    }

    /// An unresolved signal with initial value `init`.
    pub fn signal<'v>(
        &mut self,
        name: &str,
        ty: SignalType,
        init: impl Into<ValueRef<'v>>,
    ) -> SignalId {
        self.add_signal(name, ty, init.into(), None)
    }

    /// A resolved signal.
    pub fn resolved_signal<'v>(
        &mut self,
        name: &str,
        ty: SignalType,
        init: impl Into<ValueRef<'v>>,
        resolution: Resolution,
    ) -> SignalId {
        if matches!(resolution, Resolution::StdLogic) && ty.elem != Elem::Logic {
            self.usage(format!(
                "signal {name}: std_logic resolution needs a logic signal"
            ));
        }
        self.add_signal(name, ty, init.into(), Some(resolution))
    }

    fn add_signal(
        &mut self,
        name: &str,
        ty: SignalType,
        init: ValueRef<'_>,
        res: Option<Resolution>,
    ) -> SignalId {
        let n = ty.len as usize;
        let init = if ty.elem == Elem::Bytes(0) {
            self.usage(format!("signal {name}: element size 0"));
            Buf::zeroed(Elem::Bytes(1), n)
        } else {
            Buf::from_ref(ty.elem, n, init).unwrap_or_else(|m| {
                self.usage(format!("signal {name}: initial value: {m}"));
                Buf::zeroed(ty.elem, n)
            })
        };
        self.signals
            .push(Signal::new(name.to_string(), ty, init, res));
        SignalId(self.signals.len() as u32 - 1)
    }

    /// Resolve the unresolved signal parts `parts` together with `f`, as one composite resolved
    /// as a whole (a record or array subtype with a resolution function, LRM 4.6), possibly a
    /// subelement of a larger composite (an element of an array of resolved records: one
    /// element of each leaf). Each leaf keeps its own storage, so logic leaves stay on
    /// bit-planes; `f` sees whole composite values (see [`Resolver`](crate::Resolver)). A
    /// signal may be split into several groups, but its groups must cover it; every part must
    /// get the same sources, each covering the part, in the same order (checked by `build`),
    /// and none may be guarded.
    pub fn resolved_composite<R: Into<SigRef> + Copy>(&mut self, parts: &[R], f: crate::Resolver) {
        let g = self.groups.len() as u32;
        if parts.is_empty() {
            return self.usage("a resolved composite needs at least one leaf".into());
        }
        let mut members = Vec::with_capacity(parts.len());
        for &p in parts {
            let Some((l, r)) = self.chk(p.into()) else {
                return;
            };
            let s = &mut self.signals[l.0 as usize];
            let taken = s.groups.iter().any(|&h| {
                self.groups[h as usize]
                    .parts
                    .iter()
                    .any(|&(m, mr)| m == l.0 && mr.overlaps(r))
            });
            if s.resolution.is_some() && s.groups.is_empty() || taken {
                let msg = format!("signal {} is already resolved", s.name);
                return self.usage(msg);
            }
            s.resolution = Some(Resolution::Func(f.clone()));
            s.groups.push(g);
            members.push((l.0, r));
        }
        self.groups.push(Group { parts: members, f });
    }

    /// Mark the resolution function of `s` as touching shared state: the kernel then calls it
    /// from one thread at a time, in signal order.
    pub fn serial_resolution(&mut self, s: SignalId) {
        if let Some((s, _)) = self.chk(s.into()) {
            self.signals[s.0 as usize].serial_res = true;
        }
    }

    /// Keep only the last event of each block of 64 elements of `s` (more than 64 elements), not
    /// the history of its single elements: cheaper updates of a wide bus. `'last_event` and
    /// `'last_value` of a range that is not whole blocks then fail with an internal error, so
    /// call it only for a signal that no process asks those of. Everything else is exact.
    pub fn lean_events(&mut self, s: SignalId) {
        if let Some((s, _)) = self.chk(s.into()) {
            self.signals[s.0 as usize].set_lean_events();
        }
    }

    /// Keep no byte codes for `s`: no process reads a single element of it, so refreshing a code
    /// per changed element costs more than it saves (#734). Reads still work, from the planes.
    pub fn no_codes(&mut self, s: SignalId) {
        if let Some((s, _)) = self.chk(s.into()) {
            self.signals[s.0 as usize].set_no_codes();
        }
    }

    /// Make a resolved signal guarded (`bus` or `register`) with the disconnection time of
    /// its disconnection specification (0 if none).
    pub fn guard(&mut self, s: SignalId, kind: GuardKind, disconnect: Time) {
        let Some((s, _)) = self.chk(s.into()) else {
            return;
        };
        let sig = &mut self.signals[s.0 as usize];
        if sig.resolution.is_none() {
            let msg = format!("guarded signal {} must be resolved", sig.name);
            self.usage(msg);
            return;
        }
        sig.guard = Some(kind);
        sig.disconnect = disconnect;
    }

    fn implicit(&mut self, r: SigRef, kind: ImplicitKind) -> SignalId {
        let Some((src, range)) = self.chk(r) else {
            return self.signal("<invalid implicit>", SignalType::bytes(1, 1), &[0]);
        };
        let s = &self.signals[src.0 as usize];
        let mut name = s.name.clone();
        if range != s.full() {
            name += &format!("[{}+{}]", range.offset, range.len);
        }
        // BIT and BOOLEAN are logic with the codes of '0'/'1' (decisions.md "Interpreter:
        // BIT/BOOLEAN codes"); 'transaction starts at '0', 'stable/'quiet at TRUE.
        let bit = |b| Value::Logic(crate::value::LogicVec::new(1, crate::kernel::bit_code(b)));
        let (attr, ty, init) = match kind {
            ImplicitKind::Transaction => {
                ("transaction".to_string(), SignalType::logic(1), bit(false))
            }
            ImplicitKind::Stable(t) => (format!("stable({t})"), SignalType::logic(1), bit(true)),
            ImplicitKind::Quiet(t) => (format!("quiet({t})"), SignalType::logic(1), bit(true)),
            ImplicitKind::Delayed(t) => (
                format!("delayed({t})"),
                SignalType {
                    elem: s.ty.elem,
                    len: range.len,
                },
                s.cur
                    .extract(range.offset as usize, range.len as usize)
                    .to_value(),
            ),
        };
        let sig = self.signal(&format!("{name}'{attr}"), ty, &init);
        self.implicits.push(Implicit {
            sig,
            src,
            range,
            more: Vec::new(),
            kind,
            rise_at: None,
            delayed: VecDeque::new(),
        });
        sig
    }

    /// `s'transaction`: a BIT (one logic element, '0'/'1') toggling in every cycle in which `s`
    /// is active.
    pub fn transaction_signal(&mut self, s: impl Into<SigRef>) -> SignalId {
        self.implicit(s.into(), ImplicitKind::Transaction)
    }

    /// `s'stable(t)`: a BOOLEAN (one logic element, '1' is TRUE), FALSE from an event on `s`
    /// until `t` after it.
    pub fn stable_signal(&mut self, s: impl Into<SigRef>, t: Time) -> SignalId {
        self.implicit(s.into(), ImplicitKind::Stable(t))
    }

    /// `s'quiet(t)`: like `'stable`, but on activity instead of events.
    pub fn quiet_signal(&mut self, s: impl Into<SigRef>, t: Time) -> SignalId {
        self.implicit(s.into(), ImplicitKind::Quiet(t))
    }

    /// `s'delayed(t)`: the value of `s`, transport-delayed by `t`.
    pub fn delayed_signal(&mut self, s: impl Into<SigRef>, t: Time) -> SignalId {
        self.implicit(s.into(), ImplicitKind::Delayed(t))
    }

    /// Another source of implicit signal `sig` (`'stable`, `'quiet` or `'transaction` of a
    /// composite whose scalars are in several signals): an event or activity on `src` counts
    /// like one on the first source.
    pub fn implicit_source(&mut self, sig: SignalId, src: impl Into<SigRef>) {
        let Some(r) = self.chk(src.into()) else {
            return;
        };
        match self.implicits.iter_mut().find(|im| im.sig == sig) {
            Some(im) if !matches!(im.kind, ImplicitKind::Delayed(_)) => im.more.push(r),
            _ => self.usage(format!(
                "signal {} is not a 'stable, 'quiet or 'transaction signal",
                sig.0
            )),
        }
    }

    /// A driver for `target`, to be owned by the next process that lists it.
    pub fn driver(&mut self, target: impl Into<SigRef>) -> DriverId {
        let (s, range) = self
            .chk(target.into())
            .unwrap_or((SignalId(0), Range::new(0, 0)));
        self.drivers.push((s, range, None));
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
        self.process_boxed(name, flags, drivers, Box::new(process))
    }

    /// [`Builder::process`] for a process that is already boxed (no second box and no second
    /// virtual call per activation).
    pub fn process_boxed(
        &mut self,
        name: &str,
        flags: ProcFlags,
        drivers: &[DriverId],
        process: Box<dyn Process>,
    ) -> ProcId {
        let pid = ProcId(self.procs.len() as u32);
        for d in drivers {
            match self.drivers.get_mut(d.0 as usize).map(|d| &mut d.2) {
                None => self.usage(format!("process {name}: no driver {}", d.0)),
                Some(Some(_)) => {
                    self.usage(format!("process {name}: driver {} already owned", d.0))
                }
                Some(owner) => *owner = Some(pid),
            }
        }
        let flags = ProcFlags {
            serial: flags.serial || process.serial(),
            several_waits: flags.several_waits || process.several_waits(),
            concurrent_call: flags.concurrent_call || process.concurrent_call(),
            ..flags
        };
        self.procs.push(ProcDecl {
            name: name.to_string(),
            flags,
            process,
            drivers: drivers.to_vec(),
            trigger: None,
            idle: None,
            guard: None,
        });
        pid
    }

    /// Process `p` resumes for an event only when `t` holds ([`EdgeTrigger`]); the caller
    /// guarantees the process has that shape and waits on that element alone.
    pub fn edge_trigger(&mut self, p: ProcId, t: EdgeTrigger) {
        let logic = |s: SignalId, e: u32| {
            (self.signals.get(s.0 as usize))
                .is_some_and(|s| s.ty.elem == crate::value::Elem::Logic && e < s.ty.len)
        };
        let ok = logic(t.signal, t.elem) && t.reset.is_none_or(|(s, e, _)| logic(s, e));
        match self.procs.get_mut(p.0 as usize) {
            Some(d) if ok => d.trigger = Some(t),
            _ => self.usage(format!("edge trigger {t:?} on process {}", p.0)),
        }
    }

    /// Process `p` does nothing but assign signals without delay as a function of `reads` (and
    /// constants) when it runs. The kernel may then leave it suspended when it is woken and
    /// none of those had an event since its last activation (`Config::idle_skip`; a clocked
    /// process, with an edge trigger, at an edge); the caller guarantees the shape
    /// (`Db::pure_clocked_reads`, `Db::pure_comb_reads`, `Db::pure_conc_reads`).
    pub fn idle_skip(&mut self, p: ProcId, reads: &[(SignalId, Range)]) {
        let reads = reads
            .iter()
            .map(|&(signal, range)| IdleRead {
                signal,
                range,
                guard: None,
            })
            .collect();
        self.idle_skip_guarded(p, reads);
    }

    /// [`Builder::idle_skip`] where a read can have a condition (`IdleRead::guard`): an event
    /// on the signal then matters only while the condition holds.
    pub fn idle_skip_guarded(&mut self, p: ProcId, reads: Vec<IdleRead>) {
        match self.procs.get_mut(p.0 as usize) {
            Some(d) => d.idle = Some(reads),
            None => self.usage(format!("idle skip of process {} that does not exist", p.0)),
        }
    }

    /// Process `p` is the `_guard` process of a guarded block: `GUARD <= condition`, then a wait on
    /// the condition's signals `reads`. The kernel runs it in the update phase of the cycle in
    /// which one of them has an event, so GUARD changes in that cycle (LRM 11.2), instead of
    /// waking it like a process (GUARD would change a delta later).
    pub fn guard_process(&mut self, p: ProcId, reads: &[(SignalId, Range)]) {
        match self.procs.get_mut(p.0 as usize) {
            Some(d) => d.guard = Some(reads.to_vec()),
            None => self.usage(format!("guard process {} does not exist", p.0)),
        }
    }

    /// An uncollapsed port association between a formal and an actual (rule 3). Driving
    /// values flow up from `out`/`inout`/`buffer` formals and effective values down to
    /// `in`/`inout` formals in the update phase, without a delta.
    pub fn port(
        &mut self,
        formal: impl Into<SigRef>,
        actual: impl Into<SigRef>,
        mode: PortMode,
        conv: PortConv,
    ) {
        self.port_parts(&[formal.into()], &[actual.into()], mode, conv);
    }

    /// [`Builder::port`] for a formal and an actual made of several parts, possibly in
    /// different signals (the leaves of a record port with a conversion function). Every
    /// direction the mode uses needs a conversion unless both sides are one part. The
    /// conversion sees all source parts and writes all target parts in the same update phase
    /// (no delta), once per cycle in which any source part is active.
    pub fn port_parts(
        &mut self,
        formal: &[SigRef],
        actual: &[SigRef],
        mode: PortMode,
        conv: PortConv,
    ) {
        let mut parts = |refs: &[SigRef]| -> Option<Vec<(SignalId, Range)>> {
            refs.iter().map(|&r| self.chk(r)).collect()
        };
        let (Some(fs), Some(as_)) = (parts(formal), parts(actual)) else {
            return;
        };
        let name = |b: &Builder, p: &[(SignalId, Range)]| {
            p.first().map_or("<no parts>".into(), |(s, _)| {
                b.signals[s.0 as usize].name.clone()
            })
        };
        if fs.is_empty() || as_.is_empty() {
            let msg = format!("port {}: a side without parts", name(self, &fs));
            return self.usage(msg);
        }
        let needs = match mode {
            PortMode::In => conv.to_formal.is_none(),
            PortMode::Out | PortMode::Buffer => conv.to_actual.is_none(),
            PortMode::Inout => conv.to_formal.is_none() || conv.to_actual.is_none(),
        };
        if needs {
            // No conversion: the parts must line up one to one (an `inout` port converted
            // on the formal side only passes the actual in unchanged).
            let same = fs.len() == as_.len()
                && fs.iter().zip(&as_).all(|((f, fr), (a, ar))| {
                    self.signals[f.0 as usize].ty.elem == self.signals[a.0 as usize].ty.elem
                        && fr.len == ar.len
                });
            if !same {
                let msg = format!(
                    "port {} -> {}: types differ and no conversion is given",
                    name(self, &fs),
                    name(self, &as_)
                );
                return self.usage(msg);
            }
        }
        if fs.iter().any(|f| as_.iter().any(|a| a.0 == f.0)) {
            let msg = format!("port {} is associated with itself", name(self, &fs));
            return self.usage(msg);
        }
        self.ports.push(PortEdge {
            formal: fs,
            actual: as_,
            mode,
            conv,
            slots: Vec::new(),
            up_at: 0,
            down_at: 0,
        });
    }

    pub fn build(mut self, cfg: Config) -> Result<SimControl, SimError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let usage = |m: String| SimError::new(ErrorKind::Usage(m));
        // Per signal, the source ranges added so far (offset -> end), so the "several sources"
        // check of an unresolved signal is a lookup, not a scan over all its sources.
        let mut taken: Vec<BTreeMap<u32, u32>> = vec![BTreeMap::new(); self.signals.len()];
        let mut infos = Vec::with_capacity(self.drivers.len());
        for (i, &(sig, range, owner)) in self.drivers.iter().enumerate() {
            let owner = owner.ok_or_else(|| usage(format!("driver {i} has no process")))?;
            let local = self.procs[owner.0 as usize]
                .drivers
                .iter()
                .position(|d| d.0 == i as u32);
            let local = local.ok_or_else(|| {
                SimError::new(ErrorKind::Internal(format!("driver {i} not in its owner")))
            })?;
            let s = &mut self.signals[sig.0 as usize];
            infos.push(DriverInfo {
                owner,
                local: local as u32,
                signal: sig,
                range,
                slot: s.drivers.len() as u32,
                direct: false,
                bdirect: false,
            });
            add_source(s, range, &mut taken[sig.0 as usize]).map_err(usage)?;
        }
        for p in &mut self.ports {
            for &(f, fr) in &p.formal {
                let f = &mut self.signals[f.0 as usize];
                if matches!(p.mode, PortMode::In) && f.drivers.iter().any(|d| d.range.overlaps(fr))
                {
                    return Err(usage(format!("in-port {} has a driver", f.name)));
                }
                if matches!(p.mode, PortMode::In | PortMode::Inout) {
                    let l = Signal::layer(&mut f.port_in, &f.cur);
                    crate::value::fill_bits(&mut l.mask, fr.offset as usize, fr.len as usize, true);
                }
            }
            if !matches!(p.mode, PortMode::In) {
                for &(a_id, ar) in &p.actual {
                    let a = &mut self.signals[a_id.0 as usize];
                    p.slots.push(a.drivers.len() as u32);
                    add_source(a, ar, &mut taken[a_id.0 as usize]).map_err(usage)?;
                    if let Some(d) = a.drivers.last_mut() {
                        d.off = None; // a port source is never disconnected
                    }
                }
            }
        }
        drop(taken);
        // Port levels: every formal part deeper than every actual part.
        let mut changed = true;
        let mut passes = 0;
        while changed {
            changed = false;
            passes += 1;
            if passes > self.signals.len() + 1 {
                return Err(usage("port associations form a cycle".into()));
            }
            for p in &self.ports {
                let want = p
                    .actual
                    .iter()
                    .map(|a| self.signals[a.0.0 as usize].level + 1)
                    .max()
                    .unwrap_or(0);
                for f in &p.formal {
                    let f = &mut self.signals[f.0.0 as usize];
                    if f.level < want {
                        f.level = want;
                        changed = true;
                    }
                }
            }
        }
        // The part of each side that the update phase reaches last: driving values pop
        // deepest first (then lowest id), effective values shallowest first (then lowest id).
        // A multi-part conversion runs when that part is reached, after all others.
        let level = |s: &SignalId| self.signals[s.0 as usize].level;
        for p in &mut self.ports {
            let up = p
                .formal
                .iter()
                .map(|f| f.0)
                .max_by_key(|s| (Reverse(level(s)), s.0));
            let down = p.actual.iter().map(|a| a.0).max_by_key(|s| (level(s), s.0));
            p.up_at = up.map_or(0, |s| s.0);
            p.down_at = down.map_or(0, |s| s.0);
        }
        for s in &mut self.signals {
            s.build_segments();
            s.drivers.shrink_to_fit();
        }
        self.check_groups(&infos).map_err(usage)?;
        if cfg.skip_unchanged {
            self.mark_skippable();
        }
        let mut slots = Vec::with_capacity(self.procs.len());
        let mut procs = Vec::with_capacity(self.procs.len());
        let mut names = Vec::with_capacity(self.procs.len());
        let mut idles = Vec::with_capacity(self.procs.len());
        let mut guards = Vec::with_capacity(self.procs.len());
        for p in self.procs {
            let st = ProcState {
                waveforms: p.drivers.iter().map(|_| Waveform::default()).collect(),
                marks: (p.drivers.len() > crate::process::MARKED_DRIVERS).then(|| {
                    Box::new(crate::process::Marks {
                        epoch: 1,
                        d: vec![0; p.drivers.len()],
                    })
                }),
                known_key: crate::process::NO_EDGE,
                known: crate::process::NO_EDGE,
                wait_key: crate::process::NO_EDGE,
                ..Default::default()
            };
            slots.push(Mutex::new(Slot {
                process: p.process,
                st,
                result: Ok(()),
            }));
            let depth = p.name.trim_matches(':').matches(':').count() as u32;
            names.push(p.name);
            idles.push(p.idle);
            guards.push(p.guard);
            procs.push(ProcMeta {
                depth,
                flags: p.flags,
                generation: 0,
                ready: false,
                by_timeout: false,
                resumed: false,
                trigger: p.trigger,
                perm: 0,
                tid: 0,
                wkey: crate::process::NO_EDGE,
                idle_ok: false,
                idle_win: 0,
                idle_hits: 0,
                idle_static: false,
                idle_state: 0,
                shadow: false,
            });
        }
        SimControl::new(
            cfg,
            self.signals,
            infos,
            self.ports,
            self.implicits,
            self.groups,
            slots,
            procs,
            names,
            idles,
            guards,
        )
    }
}

impl Builder {
    /// Set `Signal::skip_ok` on the signals whose activity nothing can observe (R2b): those
    /// that are not guarded, not resolved by a user function, not in a composite group, not
    /// the source of an implicit signal, and not connected through ports to such a signal or
    /// through a conversion (activity passes through ports and runs conversions).
    fn mark_skippable(&mut self) {
        fn root(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let n = self.signals.len();
        let mut parent: Vec<usize> = (0..n).collect();
        let mut bad = vec![false; n];
        for (i, s) in self.signals.iter().enumerate() {
            bad[i] = s.guard.is_some()
                || matches!(s.resolution, Some(Resolution::Func(_)))
                || !s.groups.is_empty();
        }
        for im in &self.implicits {
            bad[im.sig.0 as usize] = true;
            for s in std::iter::once(&im.src).chain(im.more.iter().map(|(s, _)| s)) {
                bad[s.0 as usize] = true;
            }
        }
        for p in &self.ports {
            let ids: Vec<usize> = (p.formal.iter().chain(&p.actual))
                .map(|(s, _)| s.0 as usize)
                .collect();
            let conv = p.conv.to_formal.is_some() || p.conv.to_actual.is_some();
            for &i in &ids {
                bad[i] |= conv;
                let (a, b) = (root(&mut parent, ids[0]), root(&mut parent, i));
                parent[b] = a;
            }
        }
        for i in 0..n {
            if bad[i] {
                let r = root(&mut parent, i);
                bad[r] = true;
            }
        }
        for i in 0..n {
            let r = root(&mut parent, i);
            self.signals[i].skip_ok = !bad[r] && !bad[i];
        }
    }

    /// The parts of each resolved composite share their sources: same owners in the same
    /// order (a process driver or a port), each covering the part, and the same port level;
    /// the groups of a signal cover it.
    fn check_groups(&self, infos: &[DriverInfo]) -> Result<(), String> {
        let owner = |s: u32, slot: usize| {
            infos
                .iter()
                .find(|d| d.signal.0 == s && d.slot as usize == slot)
                .map(|d| d.owner)
        };
        let owners = |s: u32, r: Range| -> Option<Vec<_>> {
            let sig = &self.signals[s as usize];
            let mut v = Vec::new();
            for (i, d) in sig.drivers.iter().enumerate() {
                if d.range.overlaps(r) {
                    if !d.range.contains(r) {
                        return None;
                    }
                    v.push(owner(s, i));
                }
            }
            Some(v)
        };
        for g in &self.groups {
            let (l0, r0) = g.parts[0];
            let lead = &self.signals[l0 as usize];
            let want = owners(l0, r0);
            for &(l, r) in &g.parts {
                let s = &self.signals[l as usize];
                let bad = s.guard.is_some()
                    || s.level != lead.level
                    || want.is_none()
                    || owners(l, r) != want;
                if bad {
                    return Err(format!(
                        "resolved composite {}: leaf {} does not share the sources of the \
                         composite (a guarded or partly driven composite is not implemented)",
                        lead.name, s.name
                    ));
                }
            }
        }
        for (i, s) in self.signals.iter().enumerate() {
            let covered: u32 = (s.groups.iter())
                .flat_map(|&g| &self.groups[g as usize].parts)
                .filter(|&&(l, _)| l as usize == i)
                .map(|(_, r)| r.len)
                .sum();
            if !s.groups.is_empty() && covered != s.ty.len {
                return Err(format!(
                    "signal {} is only partly resolved as a composite",
                    s.name
                ));
            }
        }
        Ok(())
    }
}

/// Add a source (driver or out-port) for `range`, checking unresolved signals have one.
fn add_source(s: &mut Signal, range: Range, taken: &mut BTreeMap<u32, u32>) -> Result<(), String> {
    if s.resolution.is_none() && range.len > 0 {
        // The source starting last before `range` ends is the only one that can overlap it.
        let hit = taken.range(..range.end()).next_back();
        if hit.is_some_and(|(_, &end)| end > range.offset) {
            return Err(format!("unresolved signal {} has several sources", s.name));
        }
        taken.insert(range.offset, range.end());
    }
    let value = s.cur.extract(range.offset as usize, range.len as usize);
    let off = s.guard.map(|_| vec![0; (range.len as usize).div_ceil(64)]);
    // Most signals have one source: do not let the first push take room for four.
    if s.drivers.is_empty() {
        s.drivers.reserve_exact(1);
    }
    s.drivers.push(DriverSlot {
        range,
        value,
        off,
        shared: false,
    });
    Ok(())
}
