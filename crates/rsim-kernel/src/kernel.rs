//! The LRM 14.7.5 simulation cycle and `SimControl`, the only API that
//! drives simulation.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;

use crate::builder::ProcFlags;
use crate::error::{ErrorKind, SimError};
use crate::process::{
    Ctx, DriverInfo, Effect, OpKind, ProcId, ProcState, Process, Severity, Shared, SignalOp,
    check_ref, make_op,
};
use crate::signal::{ForceMode, Forced, Range, SigRef, Signal, SignalId, SignalType, Waiter};
use crate::time::{Time, Unit};

/// Kernel settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Worker threads for process evaluation; 1 runs everything inline.
    pub threads: usize,
    /// Randomize process execution order with this seed. Committed output
    /// stays in elaboration order, so only order-dependent designs differ.
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

/// What an observer may do from a callback: read signals, queue forces and
/// deposits (applied in the next cycle) and end the simulation.
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

    pub fn value(&self, r: impl Into<SigRef>) -> Result<&[u8], SimError> {
        let (s, r) = check_ref(self.signals, r.into()).map_err(|k| self.err(k))?;
        Ok(&s.cur[s.bytes(r)])
    }

    pub fn signal_name(&self, s: SignalId) -> Result<&str, SimError> {
        Ok(&check_ref(self.signals, s.into())
            .map_err(|k| self.err(k))?
            .0
            .name)
    }

    pub fn force(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: &[u8],
    ) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Force(mode, value.to_vec()))
    }

    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Release(mode))
    }

    pub fn deposit(&mut self, r: impl Into<SigRef>, value: &[u8]) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Deposit(value.to_vec()))
    }

    fn op(&mut self, r: SigRef, kind: OpKind) -> Result<(), SimError> {
        let op = make_op(self.signals, r, kind).map_err(|k| self.err(k))?;
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
    fn output(&mut self, _hook: &mut Hook<'_>, _process: ProcId, _stream: u32, _bytes: &[u8]) {}
}

/// Pause or stop a running simulation from another thread. Checked between
/// simulation cycles.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Driver(u32),
    Timeout(u32, u64),
}

/// The simulation. See DESIGN.md for the cycle and the parallel commit
/// protocol.
pub struct SimControl {
    cfg: Config,
    signals: Vec<Signal>,
    drivers: Vec<DriverInfo>,
    slots: Vec<Mutex<Slot>>,
    procs: Vec<ProcMeta>,
    names: Vec<String>,
    queue: BinaryHeap<Reverse<(Time, Entry)>>,
    ops: Vec<SignalOp>,
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

impl SimControl {
    pub(crate) fn new(
        cfg: Config,
        signals: Vec<Signal>,
        drivers: Vec<DriverInfo>,
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
        Ok(SimControl {
            names: procs.iter().map(|p| p.name.clone()).collect(),
            rng: cfg.shuffle,
            cfg,
            signals,
            drivers,
            slots,
            procs,
            queue: BinaryHeap::new(),
            ops: Vec::new(),
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
        })
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
    pub fn value(&self, r: impl Into<SigRef>) -> Result<&[u8], SimError> {
        let (s, r) = self.check(r)?;
        Ok(&s.cur[s.bytes(r)])
    }

    /// Driving value, after `force out`.
    pub fn driving_value(&self, r: impl Into<SigRef>) -> Result<Vec<u8>, SimError> {
        let (s, r) = self.check(r)?;
        Ok(r.elems().flat_map(|e| s.driving_elem(e).to_vec()).collect())
    }

    // ---- control ------------------------------------------------------

    pub fn subscribe(&mut self, o: Box<dyn Observer>) {
        self.observers.push(o);
    }

    pub fn interrupt(&self) -> Interrupt {
        self.interrupt.clone()
    }

    /// Force from outside (DAP, VHPI), applied in the next cycle.
    pub fn force(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: &[u8],
    ) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Force(mode, value.to_vec()))
    }

    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Release(mode))
    }

    /// Set the driving value until a driver next updates those elements.
    pub fn deposit(&mut self, r: impl Into<SigRef>, value: &[u8]) -> Result<(), SimError> {
        self.op(r.into(), OpKind::Deposit(value.to_vec()))
    }

    fn op(&mut self, r: SigRef, kind: OpKind) -> Result<(), SimError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let op = make_op(&self.signals, r, kind).map_err(|k| self.err(k))?;
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
                if let Some(st) = sim.poll_end() {
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
            if let Some(st) = sim.poll_end() {
                return Ok(st);
            }
            match sim.next_time() {
                None => sim.end = Some(EndReason::Idle),
                Some(_) => sim.cycle()?,
            }
            Ok(sim.poll_end().unwrap_or(RunStatus::Paused))
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
                if let Some(st) = sim.poll_end() {
                    return Ok(st);
                }
                if sim.next_time() != Some(sim.now) {
                    return Ok(RunStatus::Paused);
                }
                sim.cycle()?;
            }
        })
    }

    /// Initialize on first use, run `f`, and latch errors.
    fn guard(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<RunStatus, SimError>,
    ) -> Result<RunStatus, SimError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let r = (|| {
            if !self.started {
                self.started = true;
                self.phase(Phase::StartOfSim);
                self.ready = (0..self.procs.len() as u32).collect();
                for p in &mut self.procs {
                    p.ready = true;
                }
                let all = std::mem::take(&mut self.ready);
                self.run_batch(all, false)?;
            }
            f(self)
        })();
        if let Err(e) = &r {
            self.failed = Some(e.clone());
        }
        r
    }

    /// Ended (firing `EndOfSim` once) or a pause request.
    fn poll_end(&mut self) -> Option<RunStatus> {
        let flags = self.interrupt.0.swap(0, Ordering::Relaxed);
        if flags & Interrupt::STOP != 0 {
            self.end.get_or_insert(EndReason::Requested);
        }
        if let Some(reason) = self.end {
            if !self.end_fired {
                self.end_fired = true;
                self.phase(Phase::EndOfSim);
            }
            return Some(RunStatus::Ended(reason));
        }
        (flags & Interrupt::PAUSE != 0).then_some(RunStatus::Paused)
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
            return Err(self.err(ErrorKind::Internal(
                "simulation cycle without pending activity".into(),
            )));
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
        let (now, cycle) = (self.now, self.cycle);

        // 14.7.5.3 b: update drivers due now, then forces and deposits.
        let mut touched: Vec<u32> = Vec::new();
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
                Entry::Driver(d) => {
                    let info = &self.drivers[d as usize];
                    let mut slot = lock(&self.slots[info.owner.0 as usize]);
                    let sig = &mut self.signals[info.signal.0 as usize];
                    let es = sig.ty.elem_size as usize;
                    for txn in slot.st.waveforms[info.local as usize].take_due(now) {
                        let drv = &mut sig.drivers[info.slot as usize];
                        let off = (txn.range.offset - drv.range.offset) as usize * es;
                        drv.value[off..off + txn.value.len()].copy_from_slice(&txn.value);
                        sig.mark_active(txn.range, now, cycle);
                        sig.dirty.push(txn.range);
                        touched.push(info.signal.0);
                    }
                }
            }
        }
        let mut eff: Vec<(u32, Range)> = Vec::new();
        touched.sort_unstable();
        touched.dedup();
        for &s in &touched {
            let sig = &mut self.signals[s as usize];
            for r in std::mem::take(&mut sig.dirty) {
                sig.compute_driving(r)
                    .map_err(|k| SimError::at(k, now, self.delta))?;
                eff.push((s, r));
            }
        }
        for op in std::mem::take(&mut self.ops) {
            apply_op(&mut self.signals[op.signal.0 as usize], &op, now, cycle);
            eff.push((op.signal.0, op.range));
        }
        let mut changed: Vec<u32> = Vec::new();
        for &(s, r) in &eff {
            self.signals[s as usize].update_effective(r, now, cycle);
            changed.push(s);
        }
        // 14.7.5.3 c: implicit 'transaction signals of active signals.
        for &(s, _) in &eff {
            if let Some(t) = self.signals[s as usize].transaction
                && self.signals[t.0 as usize].active_cycle[0] != cycle
            {
                let ts = &mut self.signals[t.0 as usize];
                ts.driving[0] ^= 1;
                ts.mark_active(ts.full(), now, cycle);
                ts.update_effective(ts.full(), now, cycle);
                changed.push(t.0);
            }
        }
        changed.sort_unstable();
        changed.dedup();
        changed.retain(|&s| !self.signals[s as usize].changed.is_empty());

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
            let runs = std::mem::take(&mut sig.changed);
            let procs = &mut self.procs;
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
            for p in woken {
                self.wake(p, false);
            }
        }

        // e: run the resumed non-postponed processes.
        let ready = std::mem::take(&mut self.ready);
        self.run_batch(ready, false)?;

        // f/g: at the last delta of this time, postponed processes.
        if self.next_time() != Some(now) {
            self.phase(Phase::LastKnownDelta);
            if self.next_time() != Some(now) {
                let postponed = std::mem::take(&mut self.postponed);
                self.run_batch(postponed, true)?;
                self.phase(Phase::EndOfTimeStep);
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

    /// Run `pids` (in parallel where allowed), then commit their effects and
    /// waits in elaboration order.
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
                // A panic in process code is a broken invariant: report it as
                // an internal error of that process, not a bare panic.
                slot.result = catch_unwind(AssertUnwindSafe(|| slot.process.run(&mut ctx)))
                    .unwrap_or_else(|panic| {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "non-string panic payload".into());
                        let mut e = SimError::at(
                            ErrorKind::Internal(format!("process panicked: {msg}")),
                            sh.now,
                            sh.delta,
                        );
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
            self.commit(ProcId(p), effects);
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
                Effect::Output(stream, bytes) => {
                    self.notify(|o, h| o.output(h, process, stream, &bytes))
                }
                Effect::Stop(code) => _ = self.end.get_or_insert(EndReason::Stop(code)),
                Effect::Finish(code) => _ = self.end.get_or_insert(EndReason::Finish(code)),
                Effect::Op(op) => self.ops.push(op),
            }
        }
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

/// Lock a process slot. Process panics are caught inside the lock, so a
/// poisoned slot cannot occur; its state would be intact anyway.
fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn apply_op(sig: &mut Signal, op: &SignalOp, now: Time, cycle: u64) {
    let n = sig.ty.len as usize;
    let bytes = sig.bytes(op.range);
    fn slot(sig: &mut Signal, mode: ForceMode) -> &mut Option<Box<Forced>> {
        match mode {
            ForceMode::Effective => &mut sig.force_eff,
            ForceMode::Driving => &mut sig.force_drv,
        }
    }
    match &op.kind {
        OpKind::Force(mode, value) => {
            let size = sig.cur.len();
            let f = slot(sig, *mode).get_or_insert_with(|| {
                Box::new(Forced {
                    mask: vec![false; n],
                    value: vec![0; size],
                })
            });
            f.mask[op.range.elems()].fill(true);
            f.value[bytes].copy_from_slice(value);
        }
        OpKind::Release(mode) => {
            if let Some(f) = slot(sig, *mode) {
                f.mask[op.range.elems()].fill(false);
            }
        }
        OpKind::Deposit(value) => sig.driving[bytes].copy_from_slice(value),
    }
    sig.mark_active(op.range, now, cycle);
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
