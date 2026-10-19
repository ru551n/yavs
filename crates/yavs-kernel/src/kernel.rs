//! The LRM 14.7.5 simulation cycle and `SimControl`, the only API that drives simulation.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;

use crate::builder::{PortConv, PortMode, ProcFlags};
use crate::error::{ErrorKind, SimError};
use crate::process::{
    Ctx, DriverInfo, Effect, OpMode, ProcId, ProcState, Process, Severity, Shared, SignalOp,
    check_ref, make_op,
};
use crate::signal::{ForceMode, Range, SigRef, Signal, SignalId, Waiter, apply_txn};
use crate::time::{Time, Unit};
use crate::value::{Buf, Elem, SignalType, Value, ValueRef, fill_bits};
use yavs_logic::StdULogic;

/// Kernel settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Worker threads for process evaluation; 1 runs everything inline.
    pub threads: usize,
    /// Randomize process execution order with this seed. Committed output stays in
    /// elaboration order, so only order-dependent designs differ.
    pub shuffle: Option<u64>,
    /// Maximum delta cycles at one time.
    pub max_deltas: u32,
    /// Time resolution limit; delays are truncated to it.
    pub resolution: Unit,
    /// A report of this severity or higher ends the simulation.
    pub stop_severity: Severity,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            threads: 1,
            shuffle: None,
            max_deltas: 10_000,
            resolution: Unit::Fs,
            stop_severity: Severity::Failure,
        }
    }
}

/// Callback points, after VHPI's `vhpiCbStartOfSimulation` and friends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Before initialization.
    StartOfSim,
    /// Initialization is complete (every process ran once); before the first cycle.
    EndOfInit,
    /// A cycle at a new time is about to update signals.
    NextTimeStep,
    /// Every cycle (after `NextTimeStep` at a new time), before its wake-ups and update.
    NextCycle,
    /// Every cycle, before its resumed non-postponed processes run (#317).
    StartOfProcesses,
    /// Every cycle, after its non-postponed processes ran: what they scheduled is not yet
    /// applied (#317). Also once after initialization's process run, before `EndOfInit`.
    EndOfProcesses,
    /// No further delta is scheduled at this time; ops queued here create one.
    LastKnownDelta,
    /// Resumed postponed processes are about to run (only when there are any, #317).
    StartOfPostponed,
    /// After postponed processes; the time step is complete.
    EndOfTimeStep,
    EndOfSim,
}

/// Why the simulation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// Nothing left to do (LRM: `Tn = TIME'HIGH`).
    Idle,
    Stop(i32),
    Finish(i32),
    /// A report reached `Config::stop_severity`.
    Severity(Severity),
    /// `Interrupt::stop` or `Hook::stop`.
    Requested,
}

/// Result of a `SimControl` run call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunStatus {
    /// The requested time or step was reached, or a pause was requested.
    Paused,
    Ended(EndReason),
}

/// A committed `report`/`assert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub process: ProcId,
    pub severity: Severity,
    pub message: String,
}

/// Counters for benchmarks and the GUI status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub cycles: u64,
    pub process_runs: u64,
    /// Signal events, counted per contiguous run of changed elements.
    pub events: u64,
}

/// A pending observer wake-up (`Hook::wake_at`). Ordered by time, then registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WakeId {
    time: Time,
    seq: u64,
}

impl WakeId {
    pub fn time(self) -> Time {
        self.time
    }
}

/// Pending wake-ups, each with the index of the observer that registered it.
#[derive(Default)]
struct Wakes {
    pending: BTreeMap<WakeId, usize>,
    next: u64,
}

/// What an observer may do from a callback: read signals, queue forces and deposits
/// (applied in the next cycle), schedule wake-ups and end the simulation.
pub struct Hook<'a> {
    now: Time,
    delta: u32,
    signals: &'a [Signal],
    ops: &'a mut Vec<SignalOp>,
    end: &'a mut Option<EndReason>,
    wakes: &'a mut Wakes,
    slots: &'a [Mutex<Slot>],
    /// Index of the observer being called.
    observer: usize,
}

impl Hook<'_> {
    pub fn now(&self) -> Time {
        self.now
    }

    pub fn delta(&self) -> u32 {
        self.delta
    }

    fn err(&self, k: ErrorKind) -> SimError {
        SimError::at(k, self.now, self.delta)
    }

    pub fn value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = check_ref(self.signals, r.into()).map_err(|k| self.err(k))?;
        Ok(s.cur.extract(r.offset as usize, r.len as usize).to_value())
    }

    pub fn signal_name(&self, s: SignalId) -> Result<&str, SimError> {
        Ok(&check_ref(self.signals, s.into())
            .map_err(|k| self.err(k))?
            .0
            .name)
    }

    pub fn force<'v>(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Force(mode), Some(value.into()))
    }

    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Release(mode), None)
    }

    pub fn deposit<'v>(
        &mut self,
        r: impl Into<SigRef>,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Deposit, Some(value.into()))
    }

    fn op(&mut self, r: SigRef, mode: OpMode, v: Option<ValueRef<'_>>) -> Result<(), SimError> {
        let op = make_op(self.signals, r, mode, v).map_err(|k| self.err(k))?;
        self.ops.push(op);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.end.get_or_insert(EndReason::Requested);
    }

    /// Schedule a cycle at absolute time `t` (`>= now`; at `now` it is the next delta) whose
    /// `Observer::wake` call goes to this observer. The simulation does not end as idle while
    /// a wake-up is pending.
    pub fn wake_at(&mut self, t: Time) -> Result<WakeId, SimError> {
        if t < self.now {
            return Err(self.err(ErrorKind::DeadlineInPast { deadline: t }));
        }
        let id = WakeId {
            time: t,
            seq: self.wakes.next,
        };
        self.wakes.next += 1;
        self.wakes.pending.insert(id, self.observer);
        Ok(id)
    }

    /// Cancel a pending wake-up; false if it already fired or was cancelled.
    pub fn cancel(&mut self, id: WakeId) -> bool {
        self.wakes.pending.remove(&id).is_some()
    }

    /// End the run with `reason` (`Stop`/`Finish` carry a status), unless it already ends.
    pub fn end(&mut self, reason: EndReason) {
        self.end.get_or_insert(reason);
    }

    /// Calls `f` with process `p` (`None` for no such process). Observers run between
    /// phases, so every process is suspended: VHPI and waves read and write its variables
    /// through [`Process::as_any`].
    pub fn with_process<R>(
        &mut self,
        p: ProcId,
        f: impl FnOnce(&mut dyn Process) -> R,
    ) -> Option<R> {
        let slot = self.slots.get(p.0 as usize)?;
        Some(f(lock(slot).process.as_mut()))
    }
}

/// A subscriber on the event bus: waves, VHPI, DAP, the GUI.
pub trait Observer: Send {
    fn phase(&mut self, _hook: &mut Hook<'_>, _phase: Phase) {}
    /// Elements `range` of `signal` changed value in this cycle.
    fn value_change(&mut self, _hook: &mut Hook<'_>, _signal: SignalId, _range: Range) {}
    fn report(&mut self, _hook: &mut Hook<'_>, _report: &Report) {}
    /// textio `OUTPUT` text.
    fn output(&mut self, _hook: &mut Hook<'_>, _process: ProcId, _bytes: &[u8]) {}
    /// A wake-up this observer scheduled with `Hook::wake_at` is due: after `NextTimeStep`
    /// (if the time advanced) and before the signal update, so forces and deposits queued
    /// here apply in this cycle.
    fn wake(&mut self, _hook: &mut Hook<'_>, _id: WakeId) {}
}

/// Pause or stop a running simulation from another thread. Checked between simulation
/// cycles.
#[derive(Clone, Default)]
pub struct Interrupt(Arc<AtomicU8>);

impl Interrupt {
    const PAUSE: u8 = 1;
    const STOP: u8 = 2;

    pub fn pause(&self) {
        self.0.fetch_or(Self::PAUSE, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.0.fetch_or(Self::STOP, Ordering::Relaxed);
    }
}

pub(crate) struct Slot {
    pub process: Box<dyn Process>,
    pub st: ProcState,
    pub result: Result<(), SimError>,
}

pub(crate) struct ProcMeta {
    pub name: String,
    pub flags: ProcFlags,
    /// Bumped on every resumption; stale waiters and timeouts carry an older one.
    pub generation: u64,
    /// Resumed but not yet executed.
    pub ready: bool,
    /// The pending resumption is a timeout (orders the batch, `SimControl::order`).
    pub by_timeout: bool,
}

/// A port association between a formal and an actual (K1), each one or more sub-element
/// ranges, in order (`Builder::port_parts`).
pub(crate) struct PortEdge {
    pub formal: Vec<(SignalId, Range)>,
    pub actual: Vec<(SignalId, Range)>,
    pub mode: PortMode,
    pub conv: PortConv,
    /// Per actual part, its source slot fed by the formal (`out`, `inout`, `buffer`).
    pub slots: Vec<u32>,
    /// The formal part (signal) whose update passes the driving values up, and the actual
    /// part whose update passes the effective values down: the one each worklist reaches
    /// last, so every other part is up to date (`Builder::build`).
    pub up_at: u32,
    pub down_at: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImplicitKind {
    Transaction,
    Stable(Time),
    Quiet(Time),
    Delayed(Time),
}

/// A kernel-maintained implicit signal (K2): no process, no extra delta.
pub(crate) struct Implicit {
    pub sig: SignalId,
    pub src: SignalId,
    pub range: Range,
    /// Further sources of `'stable`, `'quiet` and `'transaction` of a composite whose
    /// scalars are in several signals (records): an event or activity on any counts.
    pub more: Vec<(SignalId, Range)>,
    pub kind: ImplicitKind,
    /// `'stable`/`'quiet`: when the value returns to TRUE.
    pub rise_at: Option<Time>,
    /// `'delayed`: transport-delayed values.
    pub delayed: VecDeque<(Time, Buf)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Driver(u32),
    Timeout(u32, u64),
    Implicit(u32),
}

/// The simulation. See DESIGN.md for the cycle and the parallel commit protocol.
pub struct SimControl {
    cfg: Config,
    signals: Vec<Signal>,
    drivers: Vec<DriverInfo>,
    ports: Vec<PortEdge>,
    /// Per signal: edges where it is the formal of an `out`/`inout`/`buffer` port.
    ports_up: Vec<Vec<u32>>,
    /// Per signal: edges where it is the actual of an `in`/`inout` port.
    ports_down: Vec<Vec<u32>>,
    /// Per signal: no port edge and no composite group, so its update touches only itself
    /// and can run in parallel with other such signals.
    independent: Vec<bool>,
    implicits: Vec<Implicit>,
    /// Composites resolved as a whole (`Builder::resolved_composite`).
    groups: Vec<Group>,
    /// Per group: the cycle it was last resolved in (once per cycle, by its first part
    /// popped with any part dirty).
    group_cycle: Vec<u64>,
    slots: Vec<Mutex<Slot>>,
    procs: Vec<ProcMeta>,
    names: Vec<String>,
    queue: BinaryHeap<Reverse<(Time, Entry)>>,
    ops: Vec<SignalOp>,
    /// Signals with events in the current cycle.
    changed_sigs: Vec<u32>,
    ready: Vec<u32>,
    postponed: Vec<u32>,
    last_ran: Vec<u32>,
    now: Time,
    delta: u32,
    cycle: u64,
    started: bool,
    end: Option<EndReason>,
    end_fired: bool,
    failed: Option<SimError>,
    observers: Vec<Box<dyn Observer>>,
    wakes: Wakes,
    pool: Option<rayon::ThreadPool>,
    rng: Option<u64>,
    interrupt: Interrupt,
    stats: Stats,
}

/// Run a port conversion over the source parts `v` and check that its result fits the
/// target parts. Without a conversion both sides are one part of the same type (`build`).
fn convert(
    conv: Option<&crate::builder::Converter>,
    v: Vec<Buf>,
    signals: &[Signal],
    targets: &[(SignalId, Range)],
    formal: &str,
) -> Result<Vec<Buf>, ErrorKind> {
    let Some(f) = conv else { return Ok(v) };
    let fail = |message| ErrorKind::Conversion {
        formal: formal.to_string(),
        message,
    };
    let vals: Vec<Value> = v.iter().map(Buf::to_value).collect();
    let out = f(&vals).map_err(fail)?;
    if out.len() != targets.len() {
        let m = format!("{} values for {} port parts", out.len(), targets.len());
        return Err(fail(m));
    }
    out.iter()
        .zip(targets)
        .map(|(o, (s, r))| {
            Buf::from_ref(signals[s.0 as usize].ty.elem, r.len as usize, o.as_ref()).map_err(fail)
        })
        .collect()
}

/// A composite resolved as a whole: its parts (leaf, elements) in concatenation order and its
/// resolver.
pub(crate) struct Group {
    pub(crate) parts: Vec<(u32, Range)>,
    pub(crate) f: crate::Resolver,
}

impl SimControl {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cfg: Config,
        signals: Vec<Signal>,
        drivers: Vec<DriverInfo>,
        ports: Vec<PortEdge>,
        implicits: Vec<Implicit>,
        groups: Vec<Group>,
        slots: Vec<Mutex<Slot>>,
        procs: Vec<ProcMeta>,
    ) -> Result<SimControl, SimError> {
        let pool = match cfg.threads {
            0 => {
                return Err(SimError::new(ErrorKind::Usage(
                    "threads must be at least 1".into(),
                )));
            }
            1 => None,
            n => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build()
                    .map_err(|e| SimError::new(ErrorKind::Usage(format!("thread pool: {e}"))))?,
            ),
        };
        let mut ports_up = vec![Vec::new(); signals.len()];
        let mut ports_down = vec![Vec::new(); signals.len()];
        let mut independent: Vec<bool> = signals.iter().map(|s| s.groups.is_empty()).collect();
        for (i, p) in ports.iter().enumerate() {
            let i = i as u32;
            for &(f, _) in &p.formal {
                independent[f.0 as usize] = false;
                let up = &mut ports_up[f.0 as usize];
                if !p.slots.is_empty() && up.last() != Some(&i) {
                    up.push(i);
                }
            }
            for &(a, _) in &p.actual {
                independent[a.0 as usize] = false;
                let down = &mut ports_down[a.0 as usize];
                if matches!(p.mode, PortMode::In | PortMode::Inout) && down.last() != Some(&i) {
                    down.push(i);
                }
            }
        }
        let mut sim = SimControl {
            names: procs.iter().map(|p| p.name.clone()).collect(),
            rng: cfg.shuffle,
            cfg,
            signals,
            drivers,
            ports,
            ports_up,
            ports_down,
            independent,
            implicits,
            group_cycle: vec![0; groups.len()],
            groups,
            slots,
            procs,
            queue: BinaryHeap::new(),
            ops: Vec::new(),
            changed_sigs: Vec::new(),
            ready: Vec::new(),
            postponed: Vec::new(),
            last_ran: Vec::new(),
            now: Time::ZERO,
            delta: 0,
            cycle: 1,
            started: false,
            end: None,
            end_fired: false,
            failed: None,
            observers: Vec::new(),
            wakes: Wakes::default(),
            pool,
            interrupt: Interrupt::default(),
            stats: Stats::default(),
        };
        sim.settle().map_err(SimError::new)?;
        Ok(sim)
    }

    /// Initial driving and effective values (LRM 14.7.5.2), through ports, without events.
    fn settle(&mut self) -> Result<(), ErrorKind> {
        let mut order: Vec<u32> = (0..self.signals.len() as u32).collect();
        order.sort_by_key(|&s| (Reverse(self.signals[s as usize].level), s));
        let mut settled = vec![false; self.groups.len()];
        for &s in &order {
            let full = self.signals[s as usize].full();
            self.signals[s as usize].compute_driving(full)?;
            // Parts share one level and are settled in id order: the first resolves the group.
            for i in 0..self.signals[s as usize].groups.len() {
                let g = self.signals[s as usize].groups[i] as usize;
                if !std::mem::replace(&mut settled[g], true) {
                    self.resolve_group(g)?;
                }
            }
            self.push_up(s, false)?;
        }
        order.sort_by_key(|&s| (self.signals[s as usize].level, s));
        for &s in &order {
            let sig = &mut self.signals[s as usize];
            sig.cur = sig.effective_of(sig.full());
            sig.last_value = sig.cur.clone();
            self.push_down(s, false)?;
        }
        Ok(())
    }

    /// Pass the formal's driving value up to the actuals of its out-ports. With `active`,
    /// only ports whose formal elements were active this cycle, marking the actuals. Returns
    /// the signals to update next: the actuals, and the formal part of a multi-part port
    /// that passes the values up once all its parts are updated (`PortEdge::up_at`).
    fn push_up(&mut self, s: u32, active: bool) -> Result<Vec<u32>, ErrorKind> {
        let mut touched = Vec::new();
        for &e in &self.ports_up[s as usize] {
            let p = &self.ports[e as usize];
            let is_active =
                |(f, r): &(SignalId, Range)| self.signals[f.0 as usize].active_in(*r, self.cycle);
            if p.up_at != s {
                if active && p.formal.iter().any(|f| f.0.0 == s && is_active(f)) {
                    touched.push(p.up_at);
                }
                continue;
            }
            if active && !p.formal.iter().any(is_active) {
                continue;
            }
            let v = p
                .formal
                .iter()
                .map(|(f, r)| self.signals[f.0 as usize].driving_of(*r))
                .collect();
            let name = &self.signals[p.formal[0].0.0 as usize].name;
            let vs = convert(p.conv.to_actual.as_ref(), v, &self.signals, &p.actual, name)?;
            if p.slots.len() != p.actual.len() {
                return Err(ErrorKind::Internal("out-port without a source slot".into()));
            }
            for ((v, &(a, ar)), &slot) in vs.into_iter().zip(&p.actual).zip(&p.slots) {
                let actual = &mut self.signals[a.0 as usize];
                actual.drivers[slot as usize].value = v;
                if active {
                    actual.mark_active(ar, self.now, self.cycle);
                    actual.dirty.push(ar);
                }
                touched.push(a.0);
            }
        }
        Ok(touched)
    }

    /// Pass the actual's effective value down to the formals of its in-ports. Returns the
    /// signals to update next: the formals, and the actual part of a multi-part port that
    /// passes the values down once all its parts are updated (`PortEdge::down_at`).
    fn push_down(&mut self, s: u32, active: bool) -> Result<Vec<u32>, ErrorKind> {
        let mut touched = Vec::new();
        for &e in &self.ports_down[s as usize] {
            let p = &self.ports[e as usize];
            let is_active =
                |(a, r): &(SignalId, Range)| self.signals[a.0 as usize].active_in(*r, self.cycle);
            if p.down_at != s {
                if active && p.actual.iter().any(|a| a.0.0 == s && is_active(a)) {
                    touched.push(p.down_at);
                }
                continue;
            }
            if active && !p.actual.iter().any(is_active) {
                continue;
            }
            let v = p
                .actual
                .iter()
                .map(|(a, r)| {
                    self.signals[a.0 as usize]
                        .cur
                        .extract(r.offset as usize, r.len as usize)
                })
                .collect();
            let name = &self.signals[p.formal[0].0.0 as usize].name;
            let vs = convert(p.conv.to_formal.as_ref(), v, &self.signals, &p.formal, name)?;
            for (v, &(f, fr)) in vs.into_iter().zip(&p.formal) {
                let formal = &mut self.signals[f.0 as usize];
                let layer = formal.port_in.as_mut().ok_or_else(|| {
                    ErrorKind::Internal(format!("in-port {} without a port layer", formal.name))
                })?;
                layer.value.copy(fr.offset as usize, &v, 0, fr.len as usize);
                if active {
                    formal.mark_active(fr, self.now, self.cycle);
                    formal.eff_dirty.push(fr);
                }
                touched.push(f.0);
            }
        }
        Ok(touched)
    }

    // ---- introspection ------------------------------------------------

    pub fn now(&self) -> Time {
        self.now
    }

    pub fn delta(&self) -> u32 {
        self.delta
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn signal_count(&self) -> usize {
        self.signals.len()
    }

    fn err(&self, k: ErrorKind) -> SimError {
        SimError::at(k, self.now, self.delta)
    }

    fn check(&self, r: impl Into<SigRef>) -> Result<(&Signal, Range), SimError> {
        check_ref(&self.signals, r.into()).map_err(|k| self.err(k))
    }

    pub fn signal_name(&self, s: SignalId) -> Result<&str, SimError> {
        Ok(&self.check(s)?.0.name)
    }

    pub fn signal_type(&self, s: SignalId) -> Result<SignalType, SimError> {
        Ok(self.check(s)?.0.ty)
    }

    pub fn signal_id(&self, name: &str) -> Option<SignalId> {
        self.signals
            .iter()
            .position(|s| s.name == name)
            .map(|i| SignalId(i as u32))
    }

    pub fn process_count(&self) -> usize {
        self.procs.len()
    }

    pub fn process_name(&self, p: ProcId) -> Result<&str, SimError> {
        let meta = self
            .procs
            .get(p.0 as usize)
            .ok_or_else(|| self.err(ErrorKind::BadRef(format!("no process {}", p.0))))?;
        Ok(&meta.name)
    }

    /// Effective value.
    pub fn value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = self.check(r)?;
        Ok(s.cur.extract(r.offset as usize, r.len as usize).to_value())
    }

    /// Driving value, after `force out`.
    pub fn driving_value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = self.check(r)?;
        Ok(s.driving_of(r).to_value())
    }

    // ---- control ------------------------------------------------------

    pub fn subscribe(&mut self, o: Box<dyn Observer>) {
        self.observers.push(o);
    }

    pub fn interrupt(&self) -> Interrupt {
        self.interrupt.clone()
    }

    /// Force from outside (DAP, VHPI), applied in the next cycle.
    pub fn force<'v>(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Force(mode), Some(value.into()))
    }

    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Release(mode), None)
    }

    /// Set the driving value until a source next updates those elements.
    pub fn deposit<'v>(
        &mut self,
        r: impl Into<SigRef>,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        self.op(r.into(), OpMode::Deposit, Some(value.into()))
    }

    fn op(&mut self, r: SigRef, mode: OpMode, v: Option<ValueRef<'_>>) -> Result<(), SimError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let op = make_op(&self.signals, r, mode, v).map_err(|k| self.err(k))?;
        self.ops.push(op);
        Ok(())
    }

    pub fn run_for(&mut self, d: Time) -> Result<RunStatus, SimError> {
        let base = self.now;
        let limit = base
            .checked_add(d)
            .map_err(|_| self.err(ErrorKind::TimeOverflow { base, delay: d }))?;
        self.run_until(limit)
    }

    /// Run all cycles at times `<= limit`, then leave `now` at `limit`.
    pub fn run_until(&mut self, limit: Time) -> Result<RunStatus, SimError> {
        self.guard(|sim| {
            loop {
                if let Some(st) = sim.poll_end()? {
                    return Ok(st);
                }
                match sim.next_time() {
                    None => sim.end = Some(EndReason::Idle),
                    Some(t) if t > limit => {
                        if limit > sim.now {
                            sim.now = limit;
                            sim.delta = 0;
                        }
                        return Ok(RunStatus::Paused);
                    }
                    Some(_) => sim.cycle()?,
                }
            }
        })
    }

    /// Run one simulation cycle.
    pub fn step_delta(&mut self) -> Result<RunStatus, SimError> {
        self.guard(|sim| {
            if let Some(st) = sim.poll_end()? {
                return Ok(st);
            }
            match sim.next_time() {
                None => sim.end = Some(EndReason::Idle),
                Some(_) => sim.cycle()?,
            }
            Ok(sim.poll_end()?.unwrap_or(RunStatus::Paused))
        })
    }

    /// Run the next cycle and every delta after it at the same time.
    pub fn step_timestep(&mut self) -> Result<RunStatus, SimError> {
        let st = self.step_delta()?;
        if st != RunStatus::Paused {
            return Ok(st);
        }
        self.guard(|sim| {
            loop {
                if let Some(st) = sim.poll_end()? {
                    return Ok(st);
                }
                if sim.next_time() != Some(sim.now) {
                    return Ok(RunStatus::Paused);
                }
                sim.cycle()?;
            }
        })
    }

    /// Initialize on first use, run `f`, latch errors, and turn a panic in kernel code (a
    /// broken invariant) into an internal error.
    fn guard(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<RunStatus, SimError>,
    ) -> Result<RunStatus, SimError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let r = catch_unwind(AssertUnwindSafe(|| {
            if !self.started {
                self.started = true;
                self.phase(Phase::StartOfSim);
                for p in &mut self.procs {
                    p.ready = true;
                }
                self.run_batch((0..self.procs.len() as u32).collect(), false, true)?;
                self.phase(Phase::EndOfProcesses);
                self.phase(Phase::EndOfInit);
                if self.end.is_none() {
                    self.last_delta()?;
                }
            }
            f(self)
        }))
        .unwrap_or_else(|panic| {
            let msg = panic_message(&*panic);
            Err(self.err(ErrorKind::Internal(format!("kernel panicked: {msg}"))))
        });
        if let Err(e) = &r {
            self.failed = Some(e.clone());
        }
        r
    }

    /// Ended (firing `EndOfSim` once) or a pause request.
    fn poll_end(&mut self) -> Result<Option<RunStatus>, SimError> {
        let flags = self.interrupt.0.swap(0, Ordering::Relaxed);
        if flags & Interrupt::STOP != 0 {
            self.end.get_or_insert(EndReason::Requested);
        }
        if let Some(reason) = self.end {
            if !self.end_fired {
                self.end_fired = true;
                self.phase(Phase::EndOfSim);
            }
            return Ok(Some(RunStatus::Ended(reason)));
        }
        Ok((flags & Interrupt::PAUSE != 0).then_some(RunStatus::Paused))
    }

    // ---- the cycle ----------------------------------------------------

    /// `Tn`: the time of the next cycle, dropping stale queue entries. Pending observer
    /// wake-ups count, so the simulation is not idle while one waits.
    fn next_time(&mut self) -> Option<Time> {
        if !self.ops.is_empty() {
            return Some(self.now);
        }
        let wake = self.wakes.pending.first_key_value().map(|(id, _)| id.time);
        match (self.next_queued(), wake) {
            (Some(q), Some(w)) => Some(q.min(w)),
            (q, w) => q.or(w),
        }
    }

    fn next_queued(&mut self) -> Option<Time> {
        while let Some(&Reverse((t, e))) = self.queue.peek() {
            let live = match e {
                Entry::Timeout(p, g) => self.procs[p as usize].generation == g,
                Entry::Driver(d) => {
                    let info = &self.drivers[d as usize];
                    let slot = lock(&self.slots[info.owner.0 as usize]);
                    slot.st.waveforms[info.local as usize]
                        .pending
                        .first()
                        .is_some_and(|x| x.time == t)
                }
                Entry::Implicit(i) => {
                    let im = &self.implicits[i as usize];
                    im.rise_at == Some(t) || im.delayed.front().is_some_and(|(dt, _)| *dt == t)
                }
            };
            if live {
                return Some(t);
            }
            self.queue.pop();
        }
        None
    }

    fn cycle(&mut self) -> Result<(), SimError> {
        let Some(next) = self.next_time() else {
            return Err(self.err(ErrorKind::Internal("cycle without pending activity".into())));
        };
        if next > self.now {
            self.now = next;
            self.delta = 0;
            self.phase(Phase::NextTimeStep);
        } else {
            if self.delta >= self.cfg.max_deltas {
                let processes = self
                    .last_ran
                    .iter()
                    .map(|&p| self.names[p as usize].clone())
                    .collect();
                return Err(self.err(ErrorKind::DeltaLimit {
                    limit: self.cfg.max_deltas,
                    processes,
                }));
            }
            self.delta += 1;
        }
        self.cycle += 1;
        self.stats.cycles += 1;
        self.phase(Phase::NextCycle);
        if !self.wakes.pending.is_empty() {
            self.fire_wakes();
        }
        self.update().map_err(|k| self.err(k))?;
        let mut changed = std::mem::take(&mut self.changed_sigs);
        changed.sort_unstable();
        changed.dedup();

        // Value-change bus, then 14.7.5.3 d: resume sensitive processes.
        if !self.observers.is_empty() {
            for &s in &changed {
                for r in self.signals[s as usize].changed.clone() {
                    self.notify(|o, h| o.value_change(h, SignalId(s), r));
                }
            }
        }
        for &s in &changed {
            let sig = &mut self.signals[s as usize];
            self.stats.events += sig.changed.len() as u64;
            let mut runs = std::mem::take(&mut sig.changed);
            let procs = &self.procs;
            let mut woken = Vec::new();
            sig.waiters.retain(|w| {
                if procs[w.proc as usize].generation != w.generation {
                    return false;
                }
                if runs.iter().any(|r| r.overlaps(w.range)) {
                    woken.push(w.proc);
                    return false;
                }
                true
            });
            sig.waiters_live = sig.waiters.len();
            runs.clear();
            sig.changed = runs;
            for p in woken {
                self.wake(p, false);
            }
        }

        changed.clear();
        self.changed_sigs = changed;

        // e: run the resumed non-postponed processes.
        let ready = std::mem::take(&mut self.ready);
        self.phase(Phase::StartOfProcesses);
        self.run_batch(ready, false, false)?;
        self.phase(Phase::EndOfProcesses);

        self.last_delta()
    }

    /// f/g: at the last delta of this time, postponed processes. Also after initialization
    /// when no delta follows at time 0, so read-write and read-only observers see time 0 (#347).
    fn last_delta(&mut self) -> Result<(), SimError> {
        if self.next_time() != Some(self.now) {
            self.phase(Phase::LastKnownDelta);
            if self.next_time() != Some(self.now) {
                let postponed = std::mem::take(&mut self.postponed);
                if !postponed.is_empty() {
                    self.phase(Phase::StartOfPostponed);
                }
                self.run_batch(postponed, true, false)?;
                self.phase(Phase::EndOfTimeStep);
            }
        }
        Ok(())
    }

    /// 14.7.5.3 b and c: sources, ports, forces, effective values and implicit signals.
    /// Records events in each signal's `changed`.
    fn update(&mut self) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        // Worklists by port level; `queued` stamps keep each signal in a list once per cycle.
        let mut up: BinaryHeap<(u32, Reverse<u32>)> = BinaryHeap::new();
        let mut down: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        let mut due: BTreeSet<u32> = BTreeSet::new();
        let mut ops: BTreeMap<u32, Vec<SignalOp>> = BTreeMap::new();
        for op in std::mem::take(&mut self.ops) {
            let s = op.signal.0;
            let sig = &mut self.signals[s as usize];
            if sig.queued_up != cycle {
                sig.queued_up = cycle;
                up.push((sig.level, Reverse(s)));
            }
            ops.entry(s).or_default().push(op);
        }
        // Matured drivers of independent signals without forces or deposits, as
        // (signal, driver) in queue order.
        let mut indep: Vec<(u32, u32)> = Vec::new();
        while let Some(&Reverse((t, e))) = self.queue.peek() {
            if t > now {
                break;
            }
            self.queue.pop();
            match e {
                Entry::Timeout(p, g) => {
                    if self.procs[p as usize].generation == g {
                        self.wake(p, true);
                    }
                }
                Entry::Implicit(i) => _ = due.insert(i),
                Entry::Driver(d) => {
                    let info = &self.drivers[d as usize];
                    let s = info.signal.0;
                    if self.independent[s as usize] && !ops.contains_key(&s) {
                        indep.push((s, d));
                        continue;
                    }
                    let mut slot = lock(&self.slots[info.owner.0 as usize]);
                    let sig = &mut self.signals[info.signal.0 as usize];
                    for txn in slot.st.waveforms[info.local as usize].take_due(now) {
                        apply_txn(&mut sig.drivers[info.slot as usize], &txn);
                        sig.mark_active(txn.range, now, cycle);
                        sig.dirty.push(txn.range);
                        if sig.queued_up != cycle {
                            sig.queued_up = cycle;
                            up.push((sig.level, Reverse(info.signal.0)));
                        }
                    }
                }
            }
        }
        self.update_independent(indep)?;

        // Driving values bottom-up: formals before the actuals they feed.
        while let Some((_, Reverse(s))) = up.pop() {
            for i in 0..self.signals[s as usize].groups.len() {
                let g = self.signals[s as usize].groups[i] as usize;
                self.update_group(g, cycle, &mut up)?;
            }
            let sig = &mut self.signals[s as usize];
            let mut dirty = std::mem::take(&mut sig.dirty);
            for r in dirty.drain(..) {
                sig.compute_driving(r)?;
                sig.eff_dirty.push(r);
            }
            sig.dirty = dirty;
            for op in ops.remove(&s).unwrap_or_default() {
                apply_op(sig, &op, now, cycle)?;
            }
            if sig.queued_down != cycle {
                sig.queued_down = cycle;
                down.push(Reverse((sig.level, s)));
            }
            for a in self.push_up(s, true)? {
                let sig = &mut self.signals[a as usize];
                if sig.queued_up != cycle {
                    sig.queued_up = cycle;
                    up.push((sig.level, Reverse(a)));
                }
            }
        }
        // Effective values top-down: actuals before their in-port formals.
        while let Some(Reverse((_, s))) = down.pop() {
            let sig = &mut self.signals[s as usize];
            let mut eff = std::mem::take(&mut sig.eff_dirty);
            let mut changed = false;
            for r in eff.drain(..) {
                changed |= sig.update_effective(r, now, cycle);
            }
            sig.eff_dirty = eff;
            if changed {
                self.changed_sigs.push(s);
            }
            for f in self.push_down(s, true)? {
                let sig = &mut self.signals[f as usize];
                if sig.queued_down != cycle {
                    sig.queued_down = cycle;
                    down.push(Reverse((sig.level, f)));
                }
            }
        }
        self.update_implicit(&due)
    }

    /// Update phase of signals without ports or groups: each one only reads its own drivers'
    /// waveforms, so they are updated in parallel, one signal per task, each applying its
    /// drivers' transactions in queue order. Errors are taken in signal id order, so the
    /// result does not depend on the thread count.
    fn update_independent(&mut self, mut due: Vec<(u32, u32)>) -> Result<(), ErrorKind> {
        if due.is_empty() {
            return Ok(());
        }
        let (now, cycle) = (self.now, self.cycle);
        due.sort_by_key(|&(s, _)| s);
        let mut jobs = Vec::new();
        let mut it = self.signals.iter_mut();
        let mut next = 0;
        for run in due.chunk_by(|a, b| a.0 == b.0) {
            let s = run[0].0 as usize;
            let sig = it
                .nth(s - next)
                .ok_or_else(|| ErrorKind::Internal(format!("driver of missing signal {s}")))?;
            next = s + 1;
            jobs.push((sig, run));
        }
        let (drivers, slots) = (&self.drivers, &self.slots);
        let job = |(sig, run): &mut (&mut Signal, &[(u32, u32)])| -> Result<bool, ErrorKind> {
            for &(_, d) in run.iter() {
                let info = &drivers[d as usize];
                let mut slot = lock(&slots[info.owner.0 as usize]);
                for txn in slot.st.waveforms[info.local as usize].take_due(now) {
                    apply_txn(&mut sig.drivers[info.slot as usize], &txn);
                    sig.mark_active(txn.range, now, cycle);
                    sig.dirty.push(txn.range);
                }
            }
            let mut dirty = std::mem::take(&mut sig.dirty);
            for &r in &dirty {
                sig.compute_driving(r)?;
            }
            let mut changed = false;
            for r in dirty.drain(..) {
                changed |= sig.update_effective(r, now, cycle);
            }
            sig.dirty = dirty;
            Ok(changed)
        };
        // ponytail: fixed cut-off below which a parallel split costs more than it saves.
        let results: Vec<Result<bool, ErrorKind>> = match &self.pool {
            Some(pool) if jobs.len() >= 256 => {
                pool.install(|| jobs.par_iter_mut().map(job).collect())
            }
            _ => jobs.iter_mut().map(job).collect(),
        };
        for (run, changed) in due.chunk_by(|a, b| a.0 == b.0).zip(results) {
            if changed? {
                self.changed_sigs.push(run[0].0);
            }
        }
        Ok(())
    }

    /// The first part of a composite group popped with any part dirty resolves the whole
    /// composite, before any leaf applies its forces and deposits; all parts then continue
    /// through the worklists (they share one level, checked at build).
    fn update_group(
        &mut self,
        g: usize,
        cycle: u64,
        up: &mut BinaryHeap<(u32, Reverse<u32>)>,
    ) -> Result<(), ErrorKind> {
        let dirty = self.groups[g]
            .parts
            .iter()
            .any(|&(l, r)| (self.signals[l as usize].dirty.iter()).any(|d| d.overlaps(r)));
        if self.group_cycle[g] == cycle || !dirty {
            return Ok(());
        }
        self.group_cycle[g] = cycle;
        self.resolve_group(g)?;
        for &(l, r) in &self.groups[g].parts {
            let sig = &mut self.signals[l as usize];
            sig.eff_dirty.push(r);
            if sig.queued_up != cycle {
                sig.queued_up = cycle;
                up.push((sig.level, Reverse(l)));
            }
        }
        Ok(())
    }

    /// Driving values of all parts of composite group `g` from one call of its resolver.
    fn resolve_group(&mut self, g: usize) -> Result<(), ErrorKind> {
        let Group { parts, f } = &self.groups[g];
        let (l0, r0) = parts[0];
        let lead = &self.signals[l0 as usize];
        // The sources of each part, in order (the same owners for every part, checked at build).
        let srcs_of = |l: u32, r: Range| {
            let s = &self.signals[l as usize];
            (s.drivers.iter().filter(move |d| d.range.overlaps(r))).map(move |d| {
                d.value
                    .codes((r.offset - d.range.offset) as usize, r.len as usize)
            })
        };
        let n = srcs_of(l0, r0).count();
        // LRM 14.7.3.2 a: without sources the driving value stays the default value; the
        // resolution function is not called.
        if n == 0 {
            return Ok(());
        }
        let mut srcs: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut out = Vec::new();
        for &(l, r) in parts {
            for (v, c) in srcs.iter_mut().zip(srcs_of(l, r)) {
                v.extend(c);
            }
            out.extend(
                self.signals[l as usize]
                    .driving
                    .codes(r.offset as usize, r.len as usize),
            );
        }
        let views: Vec<&[u8]> = srcs.iter().map(Vec::as_slice).collect();
        f(&views, &mut out).map_err(|message| ErrorKind::Resolution {
            signal: lead.name.clone(),
            message,
        })?;
        let mut at = 0;
        for &(l, r) in &self.groups[g].parts {
            let s = &mut self.signals[l as usize];
            let n = r.len as usize
                * match s.ty.elem {
                    Elem::Logic => 1,
                    Elem::Bytes(es) => es as usize,
                };
            s.driving.set_codes(r.offset as usize, &out[at..at + n]);
            at += n;
        }
        Ok(())
    }

    /// 14.7.5.3 c: `'transaction`, `'stable`, `'quiet` and `'delayed`, in creation order (an
    /// implicit signal of an implicit signal sees its source already updated).
    fn update_implicit(&mut self, due: &BTreeSet<u32>) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        for i in 0..self.implicits.len() {
            let im = &self.implicits[i];
            let src = &self.signals[im.src.0 as usize];
            let (mut ev, mut act) = (
                src.event_in(im.range, cycle),
                src.active_in(im.range, cycle),
            );
            for &(s, r) in &im.more {
                let s = &self.signals[s.0 as usize];
                ev |= s.event_in(r, cycle);
                act |= s.active_in(r, cycle);
            }
            let after = |t: Time| {
                now.checked_add(t).map_err(|_| ErrorKind::TimeOverflow {
                    base: now,
                    delay: t,
                })
            };
            let mut new: Option<Buf> = None;
            match im.kind {
                ImplicitKind::Transaction if act => {
                    let was =
                        self.signals[im.sig.0 as usize].cur.codes(0, 1) == [bit_code(true) as u8];
                    new = Some(bool_buf(!was));
                }
                ImplicitKind::Transaction => {}
                ImplicitKind::Stable(t) | ImplicitKind::Quiet(t) => {
                    let fire = if matches!(im.kind, ImplicitKind::Stable(_)) {
                        ev
                    } else {
                        act
                    };
                    let im = &mut self.implicits[i];
                    if fire {
                        let at = after(t)?;
                        im.rise_at = Some(at);
                        self.queue.push(Reverse((at, Entry::Implicit(i as u32))));
                        new = Some(bool_buf(false));
                    } else if due.contains(&(i as u32)) && im.rise_at == Some(now) {
                        im.rise_at = None;
                        new = Some(bool_buf(true));
                    }
                }
                ImplicitKind::Delayed(t) => {
                    let cur = ev.then(|| {
                        src.cur
                            .extract(im.range.offset as usize, im.range.len as usize)
                    });
                    let im = &mut self.implicits[i];
                    while im.delayed.front().is_some_and(|(dt, _)| *dt <= now) {
                        new = im.delayed.pop_front().map(|(_, v)| v);
                    }
                    if let Some(v) = cur {
                        let at = after(t)?;
                        while im.delayed.back().is_some_and(|(dt, _)| *dt >= at) {
                            im.delayed.pop_back();
                        }
                        im.delayed.push_back((at, v));
                        self.queue.push(Reverse((at, Entry::Implicit(i as u32))));
                    }
                }
            }
            if let Some(v) = new {
                let sig = &mut self.signals[self.implicits[i].sig.0 as usize];
                let full = sig.full();
                sig.driving.copy(0, &v, 0, full.len as usize);
                sig.mark_active(full, now, cycle);
                if sig.update_effective(full, now, cycle) {
                    self.changed_sigs.push(self.implicits[i].sig.0);
                }
            }
        }
        Ok(())
    }

    fn wake(&mut self, p: u32, by_timeout: bool) {
        let meta = &mut self.procs[p as usize];
        if meta.ready {
            return;
        }
        meta.ready = true;
        meta.by_timeout = by_timeout;
        meta.generation += 1;
        lock(&self.slots[p as usize]).st.timed_out = by_timeout;
        if meta.flags.postponed {
            self.postponed.push(p);
        } else {
            self.ready.push(p);
        }
    }

    /// The order in which a batch runs (serial processes) and commits (#252, user
    /// 2026-10-19): at initialisation elaboration order; afterwards the processes with one
    /// suspension point in elaboration order, then the others in reverse elaboration order,
    /// those resumed by a signal before those resumed by a timeout.
    fn order(&self, pids: &mut [u32], init: bool) {
        if init {
            pids.sort_unstable();
            return;
        }
        pids.sort_unstable_by_key(|&p| {
            let m = &self.procs[p as usize];
            if m.flags.several_waits {
                (true, m.by_timeout, !m.flags.concurrent_call, Reverse(p))
            } else {
                (false, false, false, Reverse(u32::MAX - p))
            }
        });
    }

    /// Run `pids` (in parallel where allowed), then commit their effects and waits in
    /// `order`.
    fn run_batch(
        &mut self,
        mut pids: Vec<u32>,
        postponed: bool,
        init: bool,
    ) -> Result<(), SimError> {
        if let Some(state) = &mut self.rng {
            shuffle(&mut pids, state);
        } else {
            self.order(&mut pids, init);
        }
        {
            let sh = Shared {
                now: self.now,
                delta: self.delta,
                cycle: self.cycle,
                resolution: self.cfg.resolution,
                stop_severity: self.cfg.stop_severity,
                signals: &self.signals,
                drivers: &self.drivers,
                names: &self.names,
            };
            let slots = &self.slots;
            let run = |&p: &u32| {
                let mut guard = lock(&slots[p as usize]);
                let slot = &mut *guard;
                let mut ctx = Ctx {
                    sh: &sh,
                    pid: ProcId(p),
                    st: &mut slot.st,
                };
                // A panic in process code is a broken invariant: report it as an internal
                // error of that process, not a bare panic.
                slot.result = catch_unwind(AssertUnwindSafe(|| slot.process.run(&mut ctx)))
                    .unwrap_or_else(|panic| {
                        let msg = panic_message(&*panic);
                        let kind = ErrorKind::Internal(format!("process panicked: {msg}"));
                        let mut e = SimError::at(kind, sh.now, sh.delta);
                        e.process = Some((ProcId(p), sh.names[p as usize].clone()));
                        Err(e)
                    });
            };
            match &self.pool {
                Some(pool) => {
                    let (serial, par): (Vec<u32>, Vec<u32>) = pids
                        .iter()
                        .partition(|&&p| self.procs[p as usize].flags.serial);
                    pool.install(|| par.par_iter().for_each(run));
                    serial.iter().for_each(run);
                }
                None => pids.iter().for_each(run),
            }
        }
        self.stats.process_runs += pids.len() as u64;
        self.order(&mut pids, init);
        let mut first_err = None;
        for &p in &pids {
            self.procs[p as usize].ready = false;
            let generation = self.procs[p as usize].generation;
            let mut slot = lock(&self.slots[p as usize]);
            let st = &mut slot.st;
            let effects = std::mem::take(&mut st.effects);
            if postponed
                && (st.deadline == Some(self.now)
                    || st.scheduled.iter().any(|&(t, _)| t == self.now))
            {
                let mut e = self.err(ErrorKind::PostponedDelta);
                e.process = Some((ProcId(p), self.names[p as usize].clone()));
                first_err = Some(e);
            }
            for (t, d) in st.scheduled.drain(..) {
                self.queue.push(Reverse((t, Entry::Driver(d.0))));
            }
            if let Some(t) = st.deadline.take() {
                self.queue.push(Reverse((t, Entry::Timeout(p, generation))));
            }
            for (s, range) in st.sens.drain(..) {
                let procs = &self.procs;
                self.signals[s.0 as usize].add_waiter(
                    Waiter {
                        proc: p,
                        range,
                        generation,
                    },
                    |w| procs[w.proc as usize].generation == w.generation,
                );
            }
            let result = std::mem::replace(&mut slot.result, Ok(()));
            drop(slot);
            self.commit(ProcId(p), effects);
            if let Err(e) = result {
                first_err.get_or_insert(e);
            }
            if first_err.is_some() {
                break;
            }
        }
        pids.sort_unstable();
        self.last_ran = pids;
        first_err.map_or(Ok(()), Err)
    }

    fn commit(&mut self, process: ProcId, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Report(severity, message) => {
                    let report = Report {
                        process,
                        severity,
                        message,
                    };
                    self.notify(|o, h| o.report(h, &report));
                    if severity >= self.cfg.stop_severity {
                        self.end.get_or_insert(EndReason::Severity(severity));
                    }
                }
                Effect::Output(bytes) => self.notify(|o, h| o.output(h, process, &bytes)),
                Effect::Stop(code) => _ = self.end.get_or_insert(EndReason::Stop(code)),
                Effect::Finish(code) => _ = self.end.get_or_insert(EndReason::Finish(code)),
                Effect::Op(op) => self.ops.push(op),
            }
        }
    }

    /// Call `Observer::wake` for the wake-ups due now, in `WakeId` order. Wake-ups scheduled
    /// at `now` from these calls fire in the next delta.
    fn fire_wakes(&mut self) {
        let mut due = Vec::new();
        while let Some(e) = self.wakes.pending.first_entry() {
            if e.key().time > self.now {
                break;
            }
            due.push(e.remove_entry());
        }
        for (id, who) in due {
            self.notify(|o, h| {
                if h.observer == who {
                    o.wake(h, id);
                }
            });
        }
    }

    fn phase(&mut self, phase: Phase) {
        if self.observers.is_empty() {
            return;
        }
        self.notify(|o, h| o.phase(h, phase));
    }

    fn notify(&mut self, mut f: impl FnMut(&mut dyn Observer, &mut Hook<'_>)) {
        let mut observers = std::mem::take(&mut self.observers);
        let mut hook = Hook {
            now: self.now,
            delta: self.delta,
            signals: &self.signals,
            ops: &mut self.ops,
            end: &mut self.end,
            wakes: &mut self.wakes,
            slots: &self.slots,
            observer: 0,
        };
        for (i, o) in observers.iter_mut().enumerate() {
            hook.observer = i;
            f(o.as_mut(), &mut hook);
        }
        self.observers = observers;
    }
}

/// The std_ulogic code of BIT/BOOLEAN `b` in logic storage.
pub(crate) fn bit_code(b: bool) -> StdULogic {
    if b { StdULogic::One } else { StdULogic::Zero }
}

fn bool_buf(b: bool) -> Buf {
    let mut v = Buf::zeroed(Elem::Logic, 1);
    v.set_codes(0, &[bit_code(b) as u8]);
    v
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

/// Lock a process slot. Process panics are caught inside the lock, so a poisoned slot cannot
/// occur; its state would be intact anyway.
fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn apply_op(sig: &mut Signal, op: &SignalOp, now: Time, cycle: u64) -> Result<(), ErrorKind> {
    let (off, n) = (op.range.offset as usize, op.range.len as usize);
    match (op.mode, &op.value) {
        (OpMode::Force(m), Some(v)) => {
            let slot = match m {
                ForceMode::Effective => &mut sig.force_eff,
                ForceMode::Driving => &mut sig.force_drv,
            };
            let l = Signal::layer(slot, &sig.cur);
            fill_bits(&mut l.mask, off, n, true);
            l.value.copy(off, v, 0, n);
        }
        (OpMode::Release(m), _) => {
            let slot = match m {
                ForceMode::Effective => &mut sig.force_eff,
                ForceMode::Driving => &mut sig.force_drv,
            };
            if let Some(l) = slot {
                fill_bits(&mut l.mask, off, n, false);
            }
            // The sources' value again, not a deposit made before the force (#310).
            // ponytail: a composite group keeps it until its next resolution.
            sig.compute_driving(op.range)?;
        }
        (OpMode::Deposit, Some(v)) => sig.driving.copy(off, v, 0, n),
        _ => unreachable!("make_op attaches a value to every force and deposit"),
    }
    sig.mark_active(op.range, now, cycle);
    sig.eff_dirty.push(op.range);
    Ok(())
}

/// Fisher-Yates with splitmix64: deterministic per seed, no dependency.
fn shuffle(v: &mut [u32], state: &mut u64) {
    for i in (1..v.len()).rev() {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        v.swap(i, (z % (i as u64 + 1)) as usize);
    }
}
