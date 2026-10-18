//! The LRM 14.7.5 simulation cycle and `SimControl`, the only API that drives simulation.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;

use crate::builder::{PortConv, PortMode, ProcFlags};
use crate::error::{ErrorKind, SimError};
use crate::process::{
    Ctx, DriverInfo, Effect, OpMode, ProcId, ProcState, Process, Severity, Shared, SignalOp,
    StreamId, check_ref, make_op,
};
use crate::signal::{ForceMode, Range, SigRef, Signal, SignalId, Waiter, apply_txn};
use crate::time::{Time, Unit};
use crate::value::{Buf, SignalType, Value, ValueRef, fill_bits};

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
    /// A cycle at a new time is about to update signals.
    NextTimeStep,
    /// No further delta is scheduled at this time; ops queued here create one.
    LastKnownDelta,
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

/// What an observer may do from a callback: read signals, queue forces and deposits
/// (applied in the next cycle) and end the simulation.
pub struct Hook<'a> {
    now: Time,
    delta: u32,
    signals: &'a [Signal],
    ops: &'a mut Vec<SignalOp>,
    end: &'a mut Option<EndReason>,
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
}

/// A subscriber on the event bus: waves, VHPI, DAP, the GUI.
pub trait Observer: Send {
    fn phase(&mut self, _hook: &mut Hook<'_>, _phase: Phase) {}
    /// Elements `range` of `signal` changed value in this cycle.
    fn value_change(&mut self, _hook: &mut Hook<'_>, _signal: SignalId, _range: Range) {}
    fn report(&mut self, _hook: &mut Hook<'_>, _report: &Report) {}
    /// Output to `StreamId::OUTPUT`.
    fn output(&mut self, _hook: &mut Hook<'_>, _process: ProcId, _bytes: &[u8]) {}
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
}

/// A port association between a formal and an actual sub-element range (K1).
pub(crate) struct PortEdge {
    pub formal: SignalId,
    pub frange: Range,
    pub actual: SignalId,
    pub arange: Range,
    pub mode: PortMode,
    pub conv: PortConv,
    /// The actual's source slot fed by the formal (`out`, `inout`, `buffer`).
    pub slot: Option<u32>,
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
    implicits: Vec<Implicit>,
    slots: Vec<Mutex<Slot>>,
    procs: Vec<ProcMeta>,
    names: Vec<String>,
    queue: BinaryHeap<Reverse<(Time, Entry)>>,
    ops: Vec<SignalOp>,
    /// Signals with events in the current cycle.
    changed_sigs: Vec<u32>,
    streams: BTreeMap<StreamId, Box<dyn Write + Send>>,
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
    pool: Option<rayon::ThreadPool>,
    rng: Option<u64>,
    interrupt: Interrupt,
    stats: Stats,
}

/// Run a port conversion and check that its result fits `n` elements of the target.
fn convert(
    conv: Option<&crate::builder::Converter>,
    v: Buf,
    target: &Signal,
    n: usize,
    formal: &str,
) -> Result<Buf, ErrorKind> {
    let Some(f) = conv else { return Ok(v) };
    let fail = |message| ErrorKind::Conversion {
        formal: formal.to_string(),
        message,
    };
    let out = f(v.to_value()).map_err(fail)?;
    Buf::from_ref(target.ty.elem, n, out.as_ref()).map_err(fail)
}

impl SimControl {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cfg: Config,
        signals: Vec<Signal>,
        drivers: Vec<DriverInfo>,
        ports: Vec<PortEdge>,
        implicits: Vec<Implicit>,
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
        for (i, p) in ports.iter().enumerate() {
            if p.slot.is_some() {
                ports_up[p.formal.0 as usize].push(i as u32);
            }
            if matches!(p.mode, PortMode::In | PortMode::Inout) {
                ports_down[p.actual.0 as usize].push(i as u32);
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
            implicits,
            slots,
            procs,
            queue: BinaryHeap::new(),
            ops: Vec::new(),
            changed_sigs: Vec::new(),
            streams: BTreeMap::new(),
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
        for &s in &order {
            let full = self.signals[s as usize].full();
            self.signals[s as usize].compute_driving(full)?;
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
    /// only ports whose formal elements were active this cycle, marking the actuals.
    fn push_up(&mut self, s: u32, active: bool) -> Result<Vec<u32>, ErrorKind> {
        let mut touched = Vec::new();
        for &e in &self.ports_up[s as usize] {
            let p = &self.ports[e as usize];
            let formal = &self.signals[s as usize];
            if active && !formal.active_in(p.frange, self.cycle) {
                continue;
            }
            let v = formal.driving_of(p.frange);
            let actual = &self.signals[p.actual.0 as usize];
            let v = convert(
                p.conv.to_actual.as_ref(),
                v,
                actual,
                p.arange.len as usize,
                &formal.name,
            )?;
            let slot = p
                .slot
                .ok_or_else(|| ErrorKind::Internal("out-port without a source slot".into()))?;
            let actual = &mut self.signals[p.actual.0 as usize];
            actual.drivers[slot as usize].value = v;
            if active {
                actual.mark_active(p.arange, self.now, self.cycle);
                actual.dirty.push(p.arange);
            }
            touched.push(p.actual.0);
        }
        Ok(touched)
    }

    /// Pass the actual's effective value down to the formals of its in-ports.
    fn push_down(&mut self, s: u32, active: bool) -> Result<Vec<u32>, ErrorKind> {
        let mut touched = Vec::new();
        for &e in &self.ports_down[s as usize] {
            let p = &self.ports[e as usize];
            let actual = &self.signals[s as usize];
            if active && !actual.active_in(p.arange, self.cycle) {
                continue;
            }
            let v = actual
                .cur
                .extract(p.arange.offset as usize, p.arange.len as usize);
            let formal = &self.signals[p.formal.0 as usize];
            let v = convert(
                p.conv.to_formal.as_ref(),
                v,
                formal,
                p.frange.len as usize,
                &formal.name,
            )?;
            let formal = &mut self.signals[p.formal.0 as usize];
            let layer = formal.port_in.as_mut().ok_or_else(|| {
                ErrorKind::Internal(format!("in-port {} without a port layer", formal.name))
            })?;
            layer
                .value
                .copy(p.frange.offset as usize, &v, 0, p.frange.len as usize);
            if active {
                formal.mark_active(p.frange, self.now, self.cycle);
                formal.eff_dirty.push(p.frange);
            }
            touched.push(p.formal.0);
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
                self.run_batch((0..self.procs.len() as u32).collect(), false)?;
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

    /// Ended (flushing streams and firing `EndOfSim` once) or a pause request.
    fn poll_end(&mut self) -> Result<Option<RunStatus>, SimError> {
        let flags = self.interrupt.0.swap(0, Ordering::Relaxed);
        if flags & Interrupt::STOP != 0 {
            self.end.get_or_insert(EndReason::Requested);
        }
        if let Some(reason) = self.end {
            if !self.end_fired {
                self.end_fired = true;
                for (id, mut w) in std::mem::take(&mut self.streams) {
                    w.flush().map_err(|e| {
                        self.err(ErrorKind::Runtime(format!("flushing stream {}: {e}", id.0)))
                    })?;
                }
                self.phase(Phase::EndOfSim);
            }
            return Ok(Some(RunStatus::Ended(reason)));
        }
        Ok((flags & Interrupt::PAUSE != 0).then_some(RunStatus::Paused))
    }

    // ---- the cycle ----------------------------------------------------

    /// `Tn`: the time of the next cycle, dropping stale queue entries.
    fn next_time(&mut self) -> Option<Time> {
        if !self.ops.is_empty() {
            return Some(self.now);
        }
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
        self.run_batch(ready, false)?;

        // f/g: at the last delta of this time, postponed processes.
        if self.next_time() != Some(self.now) {
            self.phase(Phase::LastKnownDelta);
            if self.next_time() != Some(self.now) {
                let postponed = std::mem::take(&mut self.postponed);
                self.run_batch(postponed, true)?;
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

        // Driving values bottom-up: formals before the actuals they feed.
        while let Some((_, Reverse(s))) = up.pop() {
            let sig = &mut self.signals[s as usize];
            let mut dirty = std::mem::take(&mut sig.dirty);
            for r in dirty.drain(..) {
                sig.compute_driving(r)?;
                sig.eff_dirty.push(r);
            }
            sig.dirty = dirty;
            for op in ops.remove(&s).unwrap_or_default() {
                apply_op(sig, &op, now, cycle);
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

    /// 14.7.5.3 c: `'transaction`, `'stable`, `'quiet` and `'delayed`, in creation order (an
    /// implicit signal of an implicit signal sees its source already updated).
    fn update_implicit(&mut self, due: &BTreeSet<u32>) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        for i in 0..self.implicits.len() {
            let im = &self.implicits[i];
            let src = &self.signals[im.src.0 as usize];
            let (ev, act) = (
                src.event_in(im.range, cycle),
                src.active_in(im.range, cycle),
            );
            let after = |t: Time| {
                now.checked_add(t).map_err(|_| ErrorKind::TimeOverflow {
                    base: now,
                    delay: t,
                })
            };
            let mut new: Option<Buf> = None;
            match im.kind {
                ImplicitKind::Transaction if act => {
                    let was = self.signals[im.sig.0 as usize].cur.bytes(0, 1) == Some(&[1]);
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
        meta.generation += 1;
        lock(&self.slots[p as usize]).st.timed_out = by_timeout;
        if meta.flags.postponed {
            self.postponed.push(p);
        } else {
            self.ready.push(p);
        }
    }

    /// Run `pids` (in parallel where allowed), then commit their effects and waits in
    /// elaboration order.
    fn run_batch(&mut self, mut pids: Vec<u32>, postponed: bool) -> Result<(), SimError> {
        if let Some(state) = &mut self.rng {
            shuffle(&mut pids, state);
        } else {
            pids.sort_unstable();
        }
        {
            let sh = Shared {
                now: self.now,
                delta: self.delta,
                cycle: self.cycle,
                resolution: self.cfg.resolution,
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
        pids.sort_unstable();
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
            if let Err(e) = self.commit(ProcId(p), effects) {
                first_err.get_or_insert(e);
            }
            if let Err(e) = result {
                first_err.get_or_insert(e);
            }
            if first_err.is_some() {
                break;
            }
        }
        self.last_ran = pids;
        first_err.map_or(Ok(()), Err)
    }

    fn commit(&mut self, process: ProcId, effects: Vec<Effect>) -> Result<(), SimError> {
        let fail = |sim: &Self, k| {
            let mut e = sim.err(k);
            e.process = Some((process, sim.names[process.0 as usize].clone()));
            e
        };
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
                Effect::Output(StreamId::OUTPUT, bytes) => {
                    self.notify(|o, h| o.output(h, process, &bytes));
                }
                Effect::Output(id, bytes) => match self.streams.get_mut(&id) {
                    Some(w) => {
                        if let Err(err) = w.write_all(&bytes) {
                            return Err(fail(
                                self,
                                ErrorKind::Runtime(format!("file write: {err}")),
                            ));
                        }
                    }
                    None => {
                        let msg = format!("stream {} is not open", id.0);
                        return Err(fail(self, ErrorKind::BadRef(msg)));
                    }
                },
                Effect::Open(id, w) => _ = self.streams.insert(id, w),
                Effect::Close(id) => {
                    let mut w = self.streams.remove(&id).ok_or_else(|| {
                        fail(
                            self,
                            ErrorKind::BadRef(format!("stream {} is not open", id.0)),
                        )
                    })?;
                    if let Err(err) = w.flush() {
                        return Err(fail(self, ErrorKind::Runtime(format!("file close: {err}"))));
                    }
                }
                Effect::Stop(code) => _ = self.end.get_or_insert(EndReason::Stop(code)),
                Effect::Finish(code) => _ = self.end.get_or_insert(EndReason::Finish(code)),
                Effect::Op(op) => self.ops.push(op),
            }
        }
        Ok(())
    }

    fn phase(&mut self, phase: Phase) {
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
        };
        for o in &mut observers {
            f(o.as_mut(), &mut hook);
        }
        self.observers = observers;
    }
}

fn bool_buf(b: bool) -> Buf {
    Buf::Bytes {
        es: 1,
        data: vec![u8::from(b)],
    }
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

fn apply_op(sig: &mut Signal, op: &SignalOp, now: Time, cycle: u64) {
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
        }
        (OpMode::Deposit, Some(v)) => sig.driving.copy(off, v, 0, n),
        _ => unreachable!("make_op attaches a value to every force and deposit"),
    }
    sig.mark_active(op.range, now, cycle);
    sig.eff_dirty.push(op.range);
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
