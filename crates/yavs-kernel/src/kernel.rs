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
    Ctx, DriverInfo, Effect, LiveSink, OpMode, ProcId, ProcState, Process, Severity, Shared,
    SignalOp, check_ref, make_op,
};
use crate::signal::{
    DelayMode, DriverId, ForceMode, Range, Reg, SigRef, Signal, SignalId, Waiter, Waiters,
};
use crate::time::{Time, Unit};
use crate::value::{Buf, Elem, SignalType, Value, ValueRef, fill_bits};
use yavs_logic::StdULogic;

/// Smallest batch of ready processes handed to the thread pool.
const PAR_BATCH: usize = 32;

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
    /// Leave a process with an edge trigger suspended on an event that does not make its
    /// edge true (`Builder::edge_trigger`). Off where every activation is observable:
    /// statement coverage, a debugger, foreign callbacks.
    pub edge_triggers: bool,
    /// A zero-delay assignment that gives an unguarded, not user-resolved signal the value it
    /// already has, when its driver has nothing pending, queues no transaction (the delta
    /// still happens). Only set where nothing reads activity: the design uses neither
    /// `'active` nor `'last_active`, and no observer, coverage run or debugger sees it
    /// (`docs/design/performance.md` R2b).
    pub skip_unchanged: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            threads: 1,
            shuffle: None,
            max_deltas: 10_000,
            resolution: Unit::Fs,
            stop_severity: Severity::Failure,
            edge_triggers: true,
            skip_unchanged: false,
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

const ALL_PHASES: [Phase; 10] = [
    Phase::StartOfSim,
    Phase::EndOfInit,
    Phase::NextTimeStep,
    Phase::NextCycle,
    Phase::StartOfProcesses,
    Phase::EndOfProcesses,
    Phase::LastKnownDelta,
    Phase::StartOfPostponed,
    Phase::EndOfTimeStep,
    Phase::EndOfSim,
];

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
    /// The reporting process; `None` for a report of a function the kernel called itself (a
    /// resolution function or port conversion), which `context` names.
    pub process: Option<ProcId>,
    pub context: Option<String>,
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
/// (applied in the next cycle), schedule transactions on drivers, schedule wake-ups and end
/// the simulation.
pub struct Hook<'a> {
    now: Time,
    delta: u32,
    cycle: u64,
    cfg: &'a Config,
    drivers: &'a [DriverInfo],
    names: &'a [String],
    queue: &'a mut Queue,
    signals: &'a [Signal],
    ops: &'a mut Vec<SignalOp>,
    end: &'a mut Option<EndReason>,
    wakes: &'a mut Wakes,
    empty_delta: &'a mut bool,
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

    /// The name of process `p`, as reports show it.
    pub fn process_name(&self, p: ProcId) -> Option<&str> {
        self.names.get(p.0 as usize).map(String::as_str)
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

    /// A signal assignment through driver `d`, as its process would make it now
    /// ([`Ctx::assign`]): VHPI `vhpi_schedule_transaction` on a driver of a foreign
    /// architecture's process, which runs no code of its own.
    pub fn assign<'v, V: Into<ValueRef<'v>> + Copy>(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        waveform: &[(Time, V)],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        let owner = self
            .drivers
            .get(d.0 as usize)
            .ok_or_else(|| self.err(ErrorKind::InvalidAssign("no such driver".into())))?
            .owner;
        let sh = Shared {
            now: self.now,
            delta: self.delta,
            cycle: self.cycle,
            resolution: self.cfg.resolution,
            stop_severity: self.cfg.stop_severity,
            signals: self.signals,
            drivers: self.drivers,
            names: self.names,
            live: None,
        };
        // Observers run between phases: no process holds its slot.
        let mut slot = lock(&self.slots[owner.0 as usize]);
        let mut ctx = Ctx {
            sh: &sh,
            pid: owner,
            st: &mut slot.st,
            live: false,
        };
        ctx.assign(d, target, waveform, mode)?;
        for (t, d) in slot.st.scheduled.drain(..) {
            self.queue.push(self.now, t, Entry::Driver(d.0));
        }
        *self.empty_delta |= slot.st.take_phantoms();
        Ok(())
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
    /// Whether `phase` needs this observer: the kernel does not call `phase` for one that
    /// says no (the default is yes), so an observer of a few phases costs nothing in the rest.
    fn wants_phase(&self, _phase: Phase) -> bool {
        true
    }
    /// Whether this observer reads `value_change` (the default is yes).
    fn wants_value_changes(&self) -> bool {
        true
    }
    /// Whether `phase` matters to this observer only in a cycle after which a serial process
    /// ran (`ProcFlags::serial`, `Process::serial`): state only serial processes change, such
    /// as the shared heap. The kernel skips its `phase` call otherwise (the default is no).
    fn phase_after_serial(&self, _phase: Phase) -> bool {
        false
    }
    fn phase(&mut self, _hook: &mut Hook<'_>, _phase: Phase) {}
    /// Elements `range` of `signal` changed value in this cycle.
    fn value_change(&mut self, _hook: &mut Hook<'_>, _signal: SignalId, _range: Range) {}
    fn report(&mut self, _hook: &mut Hook<'_>, _report: &Report) {}
    /// A sink for the reports and `OUTPUT` text of processes that run serially: they reach it
    /// as they are made instead of when the process suspends, and this observer's `report`
    /// and `output` are not called for them (#483).
    fn live_sink(&self) -> Option<Arc<dyn LiveSink>> {
        None
    }
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
        self.0.fetch_or(Self::PAUSE, Ordering::AcqRel);
    }

    pub fn stop(&self) {
        self.0.fetch_or(Self::STOP, Ordering::AcqRel);
    }
}

pub(crate) struct Slot {
    pub process: Box<dyn Process>,
    pub st: ProcState,
    pub result: Result<(), SimError>,
    /// spike(mt): duration of the last run, when `YAVS_MT_STATS` asks for per-process times.
    pub ns: u64,
}

/// spike(mt): batch and cycle statistics written to the file `YAVS_MT_STATS` names
/// (`YAVS_MT_PROC=1`: per-process times too).
pub(crate) struct MtStats {
    out: std::io::BufWriter<std::fs::File>,
    pub per_proc: bool,
    names: Vec<String>,
    serial: Vec<bool>,
    pub proc_ns: Vec<u64>,
    pub proc_runs: Vec<u64>,
    pub t_cycle: Option<std::time::Instant>,
    pub t_upd: u64,
    pub t_wake: u64,
}

impl MtStats {
    fn new(path: std::ffi::OsString, names: &[String], procs: &[ProcMeta]) -> MtStats {
        MtStats {
            out: std::io::BufWriter::new(std::fs::File::create(path).expect("YAVS_MT_STATS file")),
            per_proc: std::env::var_os("YAVS_MT_PROC").is_some(),
            names: names.to_vec(),
            serial: procs.iter().map(|m| m.flags.serial).collect(),
            proc_ns: vec![0; procs.len()],
            proc_runs: vec![0; procs.len()],
            t_cycle: None,
            t_upd: 0,
            t_wake: 0,
        }
    }

    fn line(&mut self, l: std::fmt::Arguments<'_>) {
        use std::io::Write;
        let _ = self.out.write_fmt(l);
    }
}

impl MtStats {
    fn dump(&mut self) {
        use std::io::Write;
        for p in 0..self.names.len() {
            if self.proc_runs[p] > 0 {
                let _ = writeln!(
                    self.out,
                    "P {p} {} {} {} {}",
                    u8::from(self.serial[p]),
                    self.proc_runs[p],
                    self.proc_ns[p],
                    self.names[p]
                );
            }
        }
        let _ = self.out.flush();
    }
}

/// spike(mt): greedy list-schedule makespan of `jobs` on `n` workers, worker 0 starting at `init`.
fn greedy(jobs: &[u64], n: usize, init: u64) -> u64 {
    let mut l = vec![0u64; n];
    l[0] = init;
    for &j in jobs {
        let i = (0..n).min_by_key(|&i| l[i]).unwrap_or(0);
        l[i] += j;
    }
    l.into_iter().max().unwrap_or(0)
}

/// spike(mt): makespan of `jobs` in `n` contiguous chunks of equal count.
fn chunks(jobs: &[u64], n: usize) -> u64 {
    if jobs.is_empty() {
        return 0;
    }
    let c = jobs.len().div_ceil(n);
    jobs.chunks(c)
        .map(|c| c.iter().sum::<u64>())
        .max()
        .unwrap_or(0)
}

pub(crate) struct ProcMeta {
    pub flags: ProcFlags,
    /// Bumped on every resumption; stale waiters and timeouts carry an older one.
    pub generation: u64,
    /// Resumed but not yet executed.
    pub ready: bool,
    /// The pending resumption is a timeout (orders the batch, `SimControl::order`).
    pub by_timeout: bool,
    /// Has been resumed since initialisation (`SimControl::order`).
    pub resumed: bool,
    /// Instance nesting of the process (colons in its path name; `SimControl::order`).
    pub depth: u32,
    /// Resumes for an event only on this edge (`Builder::edge_trigger`).
    pub trigger: Option<crate::EdgeTrigger>,
    /// Epoch of the process's resident waiters (0: none): it waited for the same signals
    /// twice in a row without a timeout, so its waiters stay in their lists across wake-ups
    /// instead of being removed and added again (`SimControl::run_batch`).
    pub perm: u64,
    /// 1 + index in `SimControl::triggers` of `trigger` (0: none).
    pub tid: u32,
}

/// Whether a waiter still stands for its process: a resident one while its process keeps
/// waiting for the same, a plain one until the process is resumed.
#[inline(always)]
pub(crate) fn alive(procs: &[ProcMeta], w: &Waiter) -> bool {
    let m = &procs[w.proc as usize];
    if w.perm != 0 {
        m.perm == w.perm
    } else {
        m.generation == w.generation
    }
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

/// Pending queue entries: a heap by time for the future, and a plain list for those due at
/// the current time (zero delay: the next delta), which costs no heap operation. Both drain
/// in `(time, entry)` order.
#[derive(Default)]
struct Queue {
    later: BinaryHeap<Reverse<(Time, Entry)>>,
    /// Entries at the current time, in push order.
    next_delta: Vec<Entry>,
}

impl Queue {
    fn push(&mut self, now: Time, t: Time, e: Entry) {
        if t == now {
            self.next_delta.push(e);
        } else {
            self.later.push(Reverse((t, e)));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Driver(u32),
    Timeout(u32, u64),
    Implicit(u32),
}

/// Worklists of one update phase: signals by port level, implicit signals due, and the
/// forces and deposits by signal.
#[derive(Default)]
struct UpdateWork {
    up: BinaryHeap<(u32, Reverse<u32>)>,
    down: BinaryHeap<Reverse<(u32, u32)>>,
    due: BTreeSet<u32>,
    ops: BTreeMap<u32, Vec<SignalOp>>,
}

/// The simulation. See DESIGN.md for the cycle and the parallel commit protocol.
pub struct SimControl {
    cfg: Config,
    signals: Vec<Signal>,
    /// The processes waiting on each signal (kept apart from `signals` so the wake scan can read
    /// other signals while it edits a list).
    waiters: Vec<Waiters>,
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
    /// No process has several wait statements: a batch runs in process order.
    single_waits: bool,
    names: Vec<String>,
    queue: Queue,
    ops: Vec<SignalOp>,
    /// Signals with events in the current cycle.
    changed_sigs: Vec<u32>,
    ready: Vec<u32>,
    /// `ready` is ascending and holds no process whose place in the batch order is special
    /// (`order` has nothing to do for it).
    ready_mono: bool,
    /// Resumed processes with several suspension points: they run after the others, in an order
    /// of their own (`order`), so they wait here apart from `ready`.
    ready_several: Vec<u32>,
    /// The batch `run_batch` is about to run is known to be in order.
    batch_mono: bool,
    /// `Tn` as the last delta computed it (only a cycle sets it, only `run_until` takes it).
    known_next: Option<Option<Time>>,
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
    /// The first sink an observer offered (`Observer::live_sink`), and the observers (bits)
    /// that offered one.
    live_sink: Option<Arc<dyn LiveSink>>,
    live_mask: u64,
    /// The phases some observer wants (bit `Phase as u32`), and whether one wants value changes.
    phase_mask: u16,
    /// The phases some observer wants whether or not a serial process ran.
    phase_always: u16,
    /// Per observer: the phases it wants, and those it wants only after a serial process ran.
    obs_phases: Vec<(u16, u16)>,
    /// A serial process ran since the last `NextCycle` (true before the first one).
    serial_ran: bool,
    watch_changes: bool,
    wakes: Wakes,
    pool: Option<rayon::ThreadPool>,
    rng: Option<u64>,
    interrupt: Interrupt,
    stats: Stats,
    /// Initialisation is over: reports of resolution functions and port conversions go to
    /// `callouts` (with their context) instead of the interpreter printing them (#372).
    live: bool,
    callouts: Vec<(Severity, String, String)>,
    /// A skipped unchanged assignment (`Config::skip_unchanged`) left a delta that would have
    /// been there: the next cycle runs although it has no transaction.
    empty_delta: bool,
    /// Last epoch given to resident waiters (`ProcMeta::perm`).
    perm_epoch: u64,
    /// The worklists of `update`, empty between cycles.
    work: Option<Box<UpdateWork>>,
    /// Scratch lists of the cycle, kept to save their allocations.
    woken_buf: Vec<(u64, u32)>,
    indep_buf: Vec<(u32, u32)>,
    /// The distinct edge triggers of the processes (`ProcMeta::tid`).
    triggers: Vec<crate::EdgeTrigger>,
    /// Per trigger: the edge a signal resumes its process for, when `Ctx::edge` may take it as
    /// held (`ProcState::known_edge`).
    known_edges: Vec<Option<(SignalId, u32, bool)>>,
    mt: Option<Box<MtStats>>,
    /// spike(mt): `YAVS_MT_MODE` 0 as main, 1 no live scan without serial processes, 2 also
    /// the run phase in pieces of the slot array (`YAVS_MT_SPLIT` per thread, claimed from a
    /// counter, the kernel thread working too) with the early commit in the worker, 3 also the
    /// work-based inline/pool choice.
    mt_mode: u8,
    /// spike(mt): estimated work threshold (ns, `YAVS_MT_THR`) for mode 3, the per-process
    /// costs learnt from inline runs, pieces per thread, the probe counter.
    mt_thr: f64,
    mt_cost: Vec<f32>,
    mt_split: usize,
    mt_probe: u64,
}

/// Run a port conversion over the source parts `v` and check that its result fits the
/// target parts. Without a conversion both sides are one part of the same type (`build`).
fn convert(
    conv: Option<&crate::builder::Converter>,
    v: Vec<Buf>,
    signals: &[Signal],
    targets: &[(SignalId, Range)],
    formal: &str,
    callouts: Option<&mut Vec<(Severity, String, String)>>,
) -> Result<Vec<Buf>, ErrorKind> {
    let Some(f) = conv else { return Ok(v) };
    let fail = |message| ErrorKind::Conversion {
        formal: formal.to_string(),
        message,
    };
    let vals: Vec<Value> = v.iter().map(Buf::to_value).collect();
    let out = match callouts {
        Some(c) => {
            let (r, reps) = crate::callout::capture(|| f(&vals));
            c.extend(
                reps.into_iter()
                    .map(|(s, m)| (s, m, format!("port conversion of {formal}"))),
            );
            r
        }
        None => f(&vals),
    }
    .map_err(fail)?;
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
        mut signals: Vec<Signal>,
        drivers: Vec<DriverInfo>,
        ports: Vec<PortEdge>,
        implicits: Vec<Implicit>,
        groups: Vec<Group>,
        slots: Vec<Mutex<Slot>>,
        mut procs: Vec<ProcMeta>,
        names: Vec<String>,
    ) -> Result<SimControl, SimError> {
        let mut triggers = Vec::new();
        let mut ids = std::collections::HashMap::new();
        for m in &mut procs {
            if let Some(t) = m.trigger {
                m.tid = *ids.entry(t).or_insert_with(|| {
                    triggers.push(t);
                    triggers.len() as u32
                });
            }
        }
        let cfg_edge_triggers = cfg.edge_triggers;
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
        let mut independent: Vec<bool> = signals
            .iter()
            .map(|s| s.groups.is_empty() && !s.serial_res)
            .collect();
        for (i, p) in ports.iter().enumerate() {
            let i = i as u32;
            for &(f, _) in &p.formal {
                independent[f.0 as usize] = false;
                signals[f.0 as usize].out_port |= p.mode != PortMode::In;
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
            names,
            rng: cfg.shuffle,
            cfg,
            waiters: signals.iter().map(|s| Waiters::new(s.ty.len)).collect(),
            known_next: None,
            work: None,
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
            single_waits: procs.iter().all(|m| !m.flags.several_waits),
            procs,
            queue: Queue::default(),
            ops: Vec::new(),
            changed_sigs: Vec::new(),
            ready: Vec::new(),
            ready_mono: true,
            ready_several: Vec::new(),
            batch_mono: false,
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
            live_sink: None,
            live_mask: 0,
            phase_mask: 0,
            phase_always: 0,
            obs_phases: Vec::new(),
            serial_ran: true,
            watch_changes: false,
            wakes: Wakes::default(),
            pool,
            interrupt: Interrupt::default(),
            stats: Stats::default(),
            live: false,
            callouts: Vec::new(),
            empty_delta: false,
            perm_epoch: 0,
            woken_buf: Vec::new(),
            indep_buf: Vec::new(),
            known_edges: triggers
                .iter()
                .map(|t| {
                    (cfg_edge_triggers && t.reset.is_none() && !t.legacy)
                        .then_some((t.signal, t.elem, t.rising))
                })
                .collect(),
            triggers,
            mt: None,
            mt_mode: std::env::var("YAVS_MT_MODE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            mt_thr: std::env::var("YAVS_MT_THR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(15000.0),
            mt_cost: Vec::new(),
            mt_probe: 0,
            mt_split: std::env::var("YAVS_MT_SPLIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
        };
        sim.mt_cost = vec![200.0; sim.procs.len()];
        sim.mt = std::env::var_os("YAVS_MT_STATS")
            .map(|p| Box::new(MtStats::new(p, &sim.names, &sim.procs)));
        sim.settle().map_err(SimError::new)?;
        sim.live = true;
        sim.signals.iter_mut().for_each(|s| s.live = true);
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
            sig.sync_codes();
            sig.last_value = sig.cur.clone();
            self.push_down(s, false)?;
        }
        // `s'delayed(t)` starts at the initial value of `s`: a port's is its actual's, known
        // only now (the builder took the port's declared default).
        for i in 0..self.implicits.len() {
            let im = &self.implicits[i];
            if !matches!(im.kind, ImplicitKind::Delayed(_)) {
                continue;
            }
            let (s, r) = (im.sig.0 as usize, im.range);
            let v =
                (self.signals[im.src.0 as usize].cur).extract(r.offset as usize, r.len as usize);
            let sig = &mut self.signals[s];
            sig.driving.copy(0, &v, 0, r.len as usize);
            sig.cur = sig.effective_of(sig.full());
            sig.sync_codes();
            sig.last_value = sig.cur.clone();
            self.push_down(s as u32, false)?;
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
            let vs = convert(
                p.conv.to_actual.as_ref(),
                v,
                &self.signals,
                &p.actual,
                name,
                self.live.then_some(&mut self.callouts),
            )?;
            if p.slots.len() != p.actual.len() {
                return Err(ErrorKind::Internal("out-port without a source slot".into()));
            }
            let same = p.conv.to_actual.is_none() && p.formal.len() == p.actual.len();
            for (i, ((v, &(a, ar)), &slot)) in
                vs.into_iter().zip(&p.actual).zip(&p.slots).enumerate()
            {
                let idle = match p.formal.get(i) {
                    Some(&(f, fr)) if active && same && fr.len == ar.len => {
                        self.signals[f.0 as usize].idle_elems(fr, self.cycle)
                    }
                    _ => None,
                };
                let actual = &mut self.signals[a.0 as usize];
                actual.drivers[slot as usize].value = v;
                if active {
                    match idle {
                        Some(idle) => actual.mark_active_except(ar, &idle, self.now, self.cycle),
                        None => actual.mark_active(ar, self.now, self.cycle),
                    }
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
            let vs = convert(
                p.conv.to_formal.as_ref(),
                v,
                &self.signals,
                &p.formal,
                name,
                self.live.then_some(&mut self.callouts),
            )?;
            let same = p.conv.to_formal.is_none() && p.formal.len() == p.actual.len();
            for (i, (v, &(f, fr))) in vs.into_iter().zip(&p.formal).enumerate() {
                let idle = match p.actual.get(i) {
                    Some(&(a, ar)) if active && same && fr.len == ar.len => {
                        self.signals[a.0 as usize].idle_elems(ar, self.cycle)
                    }
                    _ => None,
                };
                let formal = &mut self.signals[f.0 as usize];
                let layer = formal.port_in.as_mut().ok_or_else(|| {
                    ErrorKind::Internal(format!("in-port {} without a port layer", formal.name))
                })?;
                layer.value.copy(fr.offset as usize, &v, 0, fr.len as usize);
                if active {
                    match idle {
                        Some(idle) => formal.mark_active_except(fr, &idle, self.now, self.cycle),
                        None => formal.mark_active(fr, self.now, self.cycle),
                    }
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
        let name = (self.names)
            .get(p.0 as usize)
            .ok_or_else(|| self.err(ErrorKind::BadRef(format!("no process {}", p.0))))?;
        Ok(name)
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
        if self.live_sink.is_none()
            && self.observers.len() < 64
            && let Some(l) = o.live_sink()
        {
            self.live_sink = Some(l);
            self.live_mask = 1 << self.observers.len();
        }
        self.observers.push(o);
        self.phase_mask = ALL_PHASES
            .iter()
            .filter(|&&p| self.observers.iter().any(|o| o.wants_phase(p)))
            .fold(0, |m, &p| m | 1 << p as u32);
        self.watch_changes = self.observers.iter().any(|o| o.wants_value_changes());
        if self.watch_changes {
            self.signals.iter_mut().for_each(|s| s.watched = true);
        }
        let bits = |o: &dyn Observer, f: &dyn Fn(&dyn Observer, Phase) -> bool| {
            ALL_PHASES
                .iter()
                .filter(|&&p| f(o, p))
                .fold(0, |m, &p| m | 1 << p as u32)
        };
        self.obs_phases = (self.observers.iter())
            .map(|o| {
                (
                    bits(o.as_ref(), &|o, p| o.wants_phase(p)),
                    bits(o.as_ref(), &|o, p| o.phase_after_serial(p)),
                )
            })
            .collect();
        self.phase_always = self.obs_phases.iter().fold(0, |m, &(w, s)| m | w & !s);
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
        let r = self.run_until_in(limit);
        if let Some(m) = &mut self.mt {
            m.dump();
        }
        r
    }

    fn run_until_in(&mut self, limit: Time) -> Result<RunStatus, SimError> {
        self.guard(|sim| {
            let mut known = None;
            loop {
                if let Some(st) = sim.poll_end()? {
                    return Ok(st);
                }
                match known.take().unwrap_or_else(|| sim.next_time()) {
                    None => sim.end = Some(EndReason::Idle),
                    Some(t) if t > limit => {
                        if limit > sim.now {
                            sim.now = limit;
                            sim.delta = 0;
                        }
                        return Ok(RunStatus::Paused);
                    }
                    Some(t) => {
                        sim.cycle(t)?;
                        known = sim.known_next.take();
                    }
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
                Some(t) => sim.cycle(t)?,
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
                sim.cycle(sim.now)?;
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
        // A plain load per delta; the locked exchange only when something was requested.
        let flags = match self.interrupt.0.load(Ordering::Acquire) {
            0 => 0,
            _ => self.interrupt.0.swap(0, Ordering::AcqRel),
        };
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
        if !self.ops.is_empty() || self.empty_delta {
            return Some(self.now);
        }
        if self.wakes.pending.is_empty() {
            return self.next_queued();
        }
        let wake = self.wakes.pending.first_key_value().map(|(id, _)| id.time);
        match (self.next_queued(), wake) {
            (Some(q), Some(w)) => Some(q.min(w)),
            (q, w) => q.or(w),
        }
    }

    fn next_queued(&mut self) -> Option<Time> {
        while let Some(&e) = self.queue.next_delta.last() {
            if self.live(self.now, e) {
                return Some(self.now);
            }
            self.queue.next_delta.pop();
        }
        while let Some(&Reverse((t, e))) = self.queue.later.peek() {
            if self.live(t, e) {
                return Some(t);
            }
            self.queue.later.pop();
        }
        None
    }

    /// Whether a queue entry at `t` still has something to do; stale ones are dropped before
    /// they could cause an empty cycle.
    fn live(&mut self, t: Time, e: Entry) -> bool {
        match e {
            Entry::Timeout(p, g) => self.procs[p as usize].generation == g,
            Entry::Driver(d) => {
                let info = &self.drivers[d as usize];
                let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                slot.st.waveforms[info.local as usize].first_time() == Some(t)
            }
            Entry::Implicit(i) => {
                let im = &self.implicits[i as usize];
                im.rise_at == Some(t) || im.delayed.front().is_some_and(|(dt, _)| *dt == t)
            }
        }
    }

    /// One cycle at `next`, the time `next_time` just returned.
    fn cycle(&mut self, next: Time) -> Result<(), SimError> {
        if next > self.now {
            self.now = next;
            self.delta = 0;
            self.phase(Phase::NextTimeStep);
        } else {
            if self.delta >= self.cfg.max_deltas {
                let mut ran = self.last_ran.clone();
                ran.sort_unstable();
                let processes = ran
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
        if let Some(m) = &mut self.mt {
            m.t_cycle = Some(std::time::Instant::now());
        }
        self.phase(Phase::NextCycle);
        if !self.wakes.pending.is_empty() {
            self.fire_wakes();
        }
        self.update().map_err(|k| self.err(k))?;
        if let Some(m) = &mut self.mt
            && let Some(t) = m.t_cycle
        {
            m.t_upd = t.elapsed().as_nanos() as u64;
        }
        self.commit_callouts();
        let mut changed = std::mem::take(&mut self.changed_sigs);
        if changed.len() > 1 {
            changed.sort_unstable();
            changed.dedup();
        }

        // Value-change bus, then 14.7.5.3 d: resume sensitive processes.
        if self.watch_changes {
            for &s in &changed {
                for r in self.signals[s as usize].changed.clone() {
                    self.notify(|o, h| o.value_change(h, SignalId(s), r));
                }
            }
        }
        let mut woken = std::mem::take(&mut self.woken_buf);
        let memo = std::cell::Cell::new((0u32, false));
        for &s in &changed {
            let sig = &mut self.signals[s as usize];
            self.stats.events += sig.changed.len() as u64 + std::mem::take(&mut sig.nruns);
            // No process ever waited for it (a signal only read by value).
            if self.waiters[s as usize].is_unwatched() {
                sig.changed.clear();
                continue;
            }
            // Update order is element order, mostly: sort only when it is not.
            if !sig.changed.is_sorted_by_key(|r| r.offset) {
                sig.changed.sort_unstable_by_key(|r| r.offset);
            }
            let waiters = &mut self.waiters[s as usize];
            let (procs, signals) = (&self.procs, &self.signals);
            let runs = &signals[s as usize].changed;
            let live = |w: &Waiter| alive(procs, w);
            // All waiters share one edge trigger: it holds for all or for none of them.
            let uniform = if self.cfg.edge_triggers {
                waiters
                    .uniform_trigger()
                    .map(|t| fires(signals, t, self.cycle))
            } else {
                None
            };
            // Otherwise the processes of one clock share a trigger: its answer is computed once.
            let hold = |w: &Waiter| {
                self.cfg.edge_triggers
                    && w.tid != 0
                    && match memo.get() {
                        (m, held) if m == w.tid => held,
                        _ => {
                            let t = self.triggers[w.tid as usize - 1];
                            let held = !fires(signals, t, self.cycle);
                            memo.set((w.tid, held));
                            held
                        }
                    }
            };
            if uniform == Some(false) {
                self.signals[s as usize].changed.clear();
                continue;
            }
            // Every waiter resident for the whole signal: the wake list is known.
            if let Some(tids) = waiters.cache_tids() {
                let held: smallvec::SmallVec<[(u32, bool); 4]> = tids
                    .iter()
                    .map(|&t| {
                        let h = t != 0
                            && self.cfg.edge_triggers
                            && !fires(signals, self.triggers[t as usize - 1], self.cycle);
                        (t, h)
                    })
                    .collect();
                self.signals[s as usize].changed.clear();
                // Falling edge of a clock: every waiter is held back, nothing to scan.
                if held.iter().all(|&(t, h)| t != 0 && h) {
                    continue;
                }
                let list = waiters.take_cache();
                if held.iter().any(|&(_, h)| h) {
                    for &(_, p, tid) in &list {
                        if tid == 0 || !held.iter().any(|&(t, h)| t == tid && h) {
                            self.wake(p, false);
                        }
                    }
                } else {
                    for &(_, p, _) in &list {
                        self.wake(p, false);
                    }
                }
                self.waiters[s as usize].restore_cache(list);
                continue;
            }
            match uniform {
                Some(_) => waiters.take_woken(runs, live, |_| false, &mut woken),
                None => waiters.take_woken(runs, live, hold, &mut woken),
            }
            self.signals[s as usize].changed.clear();
            for &(_, p) in &woken {
                self.wake(p, false);
            }
        }

        changed.clear();
        self.changed_sigs = changed;
        self.woken_buf = woken;

        // e: run the resumed non-postponed processes.
        let mut ready = std::mem::take(&mut self.ready);
        self.batch_mono = std::mem::replace(&mut self.ready_mono, true);
        if !self.ready_several.is_empty() {
            // The others come first in process order; these after them in their own order.
            let mut several = std::mem::take(&mut self.ready_several);
            if self.batch_mono {
                self.order(&mut several, false);
            }
            ready.extend_from_slice(&several);
            several.clear();
            self.ready_several = several;
        }
        if let Some(m) = &mut self.mt
            && let Some(t) = m.t_cycle
        {
            m.t_wake = t.elapsed().as_nanos() as u64 - m.t_upd;
        }
        self.phase(Phase::StartOfProcesses);
        self.run_batch(ready, false, false)?;
        if let Some(m) = &mut self.mt
            && let Some(t) = m.t_cycle.take()
        {
            let (u, w) = (m.t_upd, m.t_wake);
            m.line(format_args!(
                "C {u} {w} {}\n",
                t.elapsed().as_nanos() as u64 - u - w
            ));
        }
        self.phase(Phase::EndOfProcesses);

        self.last_delta()
    }

    /// f/g: at the last delta of this time, postponed processes. Also after initialization
    /// when no delta follows at time 0, so read-write and read-only observers see time 0 (#347).
    fn last_delta(&mut self) -> Result<(), SimError> {
        let next = self.next_time();
        // Another delta follows: nothing below runs, so `run_until` can use this answer.
        self.known_next = (next == Some(self.now)).then_some(next);
        if next != Some(self.now) {
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
        self.empty_delta = false;
        // Worklists by port level; `queued` stamps keep each signal in a list once per cycle.
        // Kept between cycles (empty here), so an empty one costs no construction and no drop.
        let mut work = self.work.take().unwrap_or_default();
        let UpdateWork { up, down, due, ops } = &mut *work;
        if !self.ops.is_empty() {
            for op in std::mem::take(&mut self.ops) {
                let s = op.signal.0;
                let sig = &mut self.signals[s as usize];
                if sig.queued_up != cycle {
                    sig.queued_up = cycle;
                    up.push((sig.level, Reverse(s)));
                }
                ops.entry(s).or_default().push(op);
            }
        }
        // Matured drivers of independent signals without forces or deposits, as
        // (signal, driver) in queue order.
        let mut indep = std::mem::take(&mut self.indep_buf);
        // Everything due now, in (time, entry) order: the heap pops in order and holds nothing
        // before `now` (time only advances to its first entry), as the delta list.
        let mut entries = std::mem::take(&mut self.queue.next_delta);
        let from_delta = entries.len();
        while let Some(&Reverse((t, e))) = self.queue.later.peek() {
            if t > now {
                break;
            }
            self.queue.later.pop();
            entries.push(e);
        }
        // The usual clocked case: every entry a driver of one independent signal, already in
        // order, and no force or deposit: apply them in place, with no job list or second pass.
        let mut fused = false;
        if ops.is_empty()
            && from_delta == entries.len()
            && let Some(&Entry::Driver(d0)) = entries.first()
        {
            let s0 = self.drivers[d0 as usize].signal.0;
            if self.independent[s0 as usize] {
                let mut prev = d0;
                fused = entries.iter().all(|e| match *e {
                    Entry::Driver(d) => {
                        let ok = d >= prev && self.drivers[d as usize].signal.0 == s0;
                        prev = d;
                        ok
                    }
                    _ => false,
                });
            }
            if fused {
                let sig = &mut self.signals[s0 as usize];
                for e in &entries {
                    if let Entry::Driver(d) = *e {
                        let info = &self.drivers[d as usize];
                        let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                        sig.apply_driver(
                            &mut slot.st.waveforms[info.local as usize],
                            info.slot,
                            now,
                            cycle,
                        );
                    }
                }
                if finish_signal(sig, now, cycle)? {
                    self.changed_sigs.push(s0);
                }
                entries.clear();
            }
        }
        if !fused && from_delta > 0 && entries.len() > 1 {
            entries.sort_unstable();
        }
        for &e in &entries {
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
                    if self.independent[s as usize] && (ops.is_empty() || !ops.contains_key(&s)) {
                        indep.push((s, d));
                        continue;
                    }
                    let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                    let sig = &mut self.signals[info.signal.0 as usize];
                    let mut ranges: smallvec::SmallVec<[Range; 4]> = smallvec::SmallVec::new();
                    slot.st.waveforms[info.local as usize].apply_due(
                        now,
                        &mut sig.drivers[info.slot as usize],
                        |r| ranges.push(r),
                    );
                    for r in ranges.drain(..) {
                        sig.mark_active(r, now, cycle);
                        sig.push_dirty(r);
                        if sig.queued_up != cycle {
                            sig.queued_up = cycle;
                            up.push((sig.level, Reverse(info.signal.0)));
                        }
                    }
                }
            }
        }
        entries.clear();
        self.queue.next_delta = entries;
        let r = self.update_independent(&mut indep);
        indep.clear();
        self.indep_buf = indep;
        r?;

        // Driving values bottom-up: formals before the actuals they feed.
        while !up.is_empty()
            && let Some((_, Reverse(s))) = up.pop()
        {
            for i in 0..self.signals[s as usize].groups.len() {
                let g = self.signals[s as usize].groups[i] as usize;
                self.update_group(g, cycle, up)?;
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
        while !down.is_empty()
            && let Some(Reverse((_, s))) = down.pop()
        {
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
        let r = if self.implicits.is_empty() {
            Ok(())
        } else {
            let r = self.update_implicit(due);
            due.clear();
            r
        };
        self.work = Some(work);
        r
    }

    /// Update phase of signals without ports or groups: each one only reads its own drivers'
    /// waveforms, so they are updated in parallel, one signal per task, each applying its
    /// drivers' transactions in queue order. Errors are taken in signal id order, so the
    /// result does not depend on the thread count.
    fn update_independent(&mut self, due: &mut [(u32, u32)]) -> Result<(), ErrorKind> {
        if due.is_empty() {
            return Ok(());
        }
        let (now, cycle) = (self.now, self.cycle);
        if due.len() > 1 {
            due.sort_by_key(|&(s, _)| s);
        }
        // The usual clocked case, every driver on one signal: no job list, no pool.
        if due.first().map(|d| d.0) == due.last().map(|d| d.0) {
            let s = due[0].0 as usize;
            let sig = self
                .signals
                .get_mut(s)
                .ok_or_else(|| ErrorKind::Internal(format!("driver of missing signal {s}")))?;
            for &(_, d) in due.iter() {
                let info = &self.drivers[d as usize];
                let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                sig.apply_driver(
                    &mut slot.st.waveforms[info.local as usize],
                    info.slot,
                    now,
                    cycle,
                );
            }
            if finish_signal(sig, now, cycle)? {
                self.changed_sigs.push(s as u32);
            }
            return Ok(());
        }
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
        let drivers = &self.drivers;
        // ponytail: fixed cut-off below which a parallel split costs more than it saves.
        let results: Vec<Result<bool, ErrorKind>> = match &self.pool {
            Some(pool) if jobs.len() >= 256 => {
                let slots = &self.slots;
                let job = |(sig, run): &mut (&mut Signal, &[(u32, u32)])| {
                    for &(_, d) in run.iter() {
                        let info = &drivers[d as usize];
                        let mut slot = lock(&slots[info.owner.0 as usize]);
                        sig.apply_driver(
                            &mut slot.st.waveforms[info.local as usize],
                            info.slot,
                            now,
                            cycle,
                        );
                    }
                    finish_signal(sig, now, cycle)
                };
                pool.install(|| jobs.par_iter_mut().map(job).collect())
            }
            // No other thread touches the slots: no lock.
            _ => {
                let slots = &mut self.slots;
                jobs.iter_mut()
                    .map(|(sig, run)| {
                        for &(_, d) in run.iter() {
                            let info = &drivers[d as usize];
                            let slot = slot_mut(&mut slots[info.owner.0 as usize]);
                            sig.apply_driver(
                                &mut slot.st.waveforms[info.local as usize],
                                info.slot,
                                now,
                                cycle,
                            );
                        }
                        finish_signal(sig, now, cycle)
                    })
                    .collect()
            }
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
        let r = if self.live {
            let (r, reps) = crate::callout::capture(|| f(&views, &mut out));
            let ctx = format!("resolution of {}", lead.name);
            (self.callouts).extend(reps.into_iter().map(|(s, m)| (s, m, ctx.clone())));
            r
        } else {
            f(&views, &mut out)
        };
        r.map_err(|message| ErrorKind::Resolution {
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
                        self.queue.push(now, at, Entry::Implicit(i as u32));
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
                        self.queue.push(now, at, Entry::Implicit(i as u32));
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

    #[inline(always)]
    fn wake(&mut self, p: u32, by_timeout: bool) {
        let meta = &mut self.procs[p as usize];
        if meta.ready {
            return;
        }
        meta.ready = true;
        meta.by_timeout = by_timeout;
        meta.generation += 1;
        if meta.flags.postponed {
            self.postponed.push(p);
        } else {
            if meta.flags.several_waits {
                self.ready_several.push(p);
            } else {
                self.ready_mono &= self.ready.last().is_none_or(|&l| l < p);
                self.ready.push(p);
            }
        }
    }

    /// The order in which a batch runs (serial processes) and commits (#252, user
    /// 2026-10-19): at initialisation elaboration order; afterwards the processes with one
    /// suspension point in elaboration order, then the others in reverse elaboration order,
    /// those resumed by a signal before those resumed by a timeout.
    fn order(&self, pids: &mut [u32], init: bool) {
        if pids.len() < 2 {
            return;
        }
        if init || self.single_waits {
            // Every key is the process number: a wake list is in this order mostly.
            if !pids.is_sorted() {
                pids.sort_unstable();
            }
            return;
        }
        // One packed word per process, looked up once (not per comparison): the flags in the
        // high bits, then the hierarchy depth (settled processes), the elaboration index
        // (reversed for several-wait processes except the settled ones) in the low 32.
        const SETTLED: u64 = 1;
        let key = |p: u32| {
            let m = &self.procs[p as usize];
            if !m.flags.several_waits {
                return u64::from(p);
            }
            // A process resumed by a signal after earlier resumptions runs in elaboration
            // order (#417), after the concurrent procedure calls (#281, #462) and with the
            // processes of deeper instances first (#469).
            let settled = m.resumed && !m.by_timeout && !m.flags.concurrent_call;
            let (rank, depth, idx) = if m.flags.concurrent_call {
                (0, 0, !p)
            } else if settled {
                (SETTLED, 0xffff - u64::from(m.depth).min(0xffff), p)
            } else {
                (2, 0, !p)
            };
            1 << 51 | u64::from(m.by_timeout) << 50 | rank << 48 | depth << 32 | u64::from(idx)
        };
        if pids.is_sorted_by_key(|&p| key(p)) {
            return;
        }
        let mut keyed: Vec<u64> = pids.iter().map(|&p| key(p)).collect();
        // Stable sort: it follows the sorted runs a batch mostly consists of.
        keyed.sort();
        for (p, k) in pids.iter_mut().zip(keyed) {
            *p = if k >> 51 != 0 && (k >> 48) & 3 != SETTLED {
                !(k as u32)
            } else {
                k as u32
            };
        }
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
        } else if !std::mem::take(&mut self.batch_mono) {
            self.order(&mut pids, init);
        }
        let t_run = self.mt.as_ref().map(|_| std::time::Instant::now());
        let per_proc = self.mt.as_ref().is_some_and(|m| m.per_proc);
        let mut pooled = false;
        let mut sub = [0u64; 3];
        let learn = self.mt_mode >= 3 && self.pool.is_some();
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
                live: self.live_sink.as_deref(),
            };
            // Reports go straight to the output when the commit order is the order they are
            // made in: no shuffled run, and (below) nothing earlier in the batch to write.
            let live_ok = self.rng.is_none() && self.live_sink.is_some();
            // An error ends the commit at its process: later ones must not write.
            let stops = |slot: &Slot| {
                slot.result.is_err()
                    || postponed
                        && (slot.st.deadline == Some(sh.now)
                            || !slot.st.phantoms.is_empty()
                            || slot.st.scheduled.iter().any(|&(t, _)| t == sh.now))
            };
            let run = |p: u32, slot: &mut Slot, live: bool| {
                let mut ctx = Ctx {
                    sh: &sh,
                    pid: ProcId(p),
                    st: &mut slot.st,
                    live,
                };
                let t = per_proc.then(std::time::Instant::now);
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
                if let Some(t) = t {
                    slot.ns = t.elapsed().as_nanos() as u64;
                }
            };
            let mt_mode = self.mt_mode;
            let est: f64 = if mt_mode >= 3 {
                {
                    // The work that can be split: the parallel processes minus the largest one.
                    let (sum, max) = pids
                        .iter()
                        .filter(|&&p| !self.procs[p as usize].flags.serial)
                        .map(|&p| f64::from(self.mt_cost[p as usize]))
                        .fold((0.0, 0.0f64), |(s, m), c| (s + c, m.max(c)));
                    sum - max
                }
            } else {
                0.0
            };
            let big = if mt_mode >= 3 {
                // Costs are learnt from inline runs only (a pooled run measures cold caches);
                // one batch in 64 that would pool runs inline to refresh them.
                {
                    self.mt_probe += 1;
                    est > self.mt_thr && self.mt_probe % 64 != 0
                }
            } else {
                pids.len() >= PAR_BATCH
            };
            let chunked = mt_mode >= 2
                && self.pool.is_some()
                && !init
                && !postponed
                && self.rng.is_none()
                && pids.len() >= 2
                && big
                && pids.iter().all(|&p| !self.procs[p as usize].flags.serial);
            match &self.pool {
                Some(pool) if chunked => {
                    pooled = true;
                    let mut sorted = pids.clone();
                    sorted.sort_unstable();
                    let k = pool.current_num_threads().min(sorted.len());
                    let pieces = (k * self.mt_split).min(sorted.len());
                    let per = sorted.len().div_ceil(pieces);
                    let mut rest_s: &mut [Mutex<Slot>] = &mut self.slots[..];
                    let mut rest_p: &mut [ProcMeta] = &mut self.procs[..];
                    let mut base = 0usize;
                    let mut parts = Vec::with_capacity(pieces);
                    for c in sorted.chunks(per) {
                        let (lo, hi) = (c[0] as usize, c[c.len() - 1] as usize + 1);
                        let (_, s) = std::mem::take(&mut rest_s).split_at_mut(lo - base);
                        let (s, r) = s.split_at_mut(hi - lo);
                        rest_s = r;
                        let (_, m) = std::mem::take(&mut rest_p).split_at_mut(lo - base);
                        let (m, r) = m.split_at_mut(hi - lo);
                        rest_p = r;
                        base = hi;
                        parts.push(Mutex::new((c, lo, s, m, Vec::<(Time, u32)>::new(), 0u64)));
                    }
                    let known_edges = &self.known_edges;
                    let next = std::sync::atomic::AtomicUsize::new(0);
                    let work = || {
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(part) = parts.get(i) else { break };
                            let mut part = part.lock().unwrap_or_else(PoisonError::into_inner);
                            let t = std::time::Instant::now();
                            let (c, lo, s, m, out, ns) = &mut *part;
                            for &p in c.iter() {
                                let i = p as usize - *lo;
                                let slot = slot_mut(&mut s[i]);
                                let meta = &mut m[i];
                                slot.st.timed_out = meta.by_timeout;
                                slot.st.known_edge =
                                    match known_edges.get((meta.tid as usize).wrapping_sub(1)) {
                                        Some(&k) if !meta.by_timeout => k,
                                        _ => None,
                                    };
                                run(p, slot, false);
                                let st = &mut slot.st;
                                if st.again
                                    && meta.perm != 0
                                    && st.sens.is_empty()
                                    && st.deadline.is_none()
                                    && st.effects.is_empty()
                                    && st.phantoms.is_empty()
                                    && slot.result.is_ok()
                                {
                                    st.again = false;
                                    meta.ready = false;
                                    meta.resumed = true;
                                    for &(t, d) in st.scheduled.iter() {
                                        out.push((t, d.0));
                                    }
                                    st.scheduled.clear();
                                }
                            }
                            *ns = t.elapsed().as_nanos() as u64;
                        }
                    };
                    pool.in_place_scope(|sc| {
                        for _ in 1..k {
                            let work = &work;
                            sc.spawn(move |_| work());
                        }
                        work();
                    });
                    let done: Vec<(Vec<u32>, Vec<(Time, u32)>, u64)> = parts
                        .into_iter()
                        .map(|p| {
                            let p = p.into_inner().unwrap_or_else(PoisonError::into_inner);
                            (p.0.to_vec(), p.4, p.5)
                        })
                        .collect();
                    let now = self.now;
                    for (_, out, _) in done {
                        for (t, d) in out {
                            self.queue.push(now, t, Entry::Driver(d));
                        }
                    }
                }
                // ponytail: fixed cut-off; below it the pool hand-off costs more than the
                // processes. Inline or pooled, the commit order is the batch order.
                Some(pool) if (mt_mode < 3 && pids.len() >= PAR_BATCH) || (mt_mode >= 3 && big) => {
                    pooled = true;
                    let ts = std::time::Instant::now();
                    let (serial, par): (Vec<u32>, Vec<u32>) = pids
                        .iter()
                        .partition(|&&p| self.procs[p as usize].flags.serial);
                    sub[0] = ts.elapsed().as_nanos() as u64;
                    let ts = std::time::Instant::now();
                    let slots = &self.slots;
                    let (procs, known_edges) = (&self.procs, &self.known_edges);
                    pool.install(|| {
                        par.par_iter().for_each(|&p| {
                            let mut s = lock(&slots[p as usize]);
                            if !init {
                                resumption(procs, known_edges, p, &mut s.st);
                            }
                            run(p, &mut s, false)
                        })
                    });
                    sub[1] = ts.elapsed().as_nanos() as u64;
                    let ts = std::time::Instant::now();
                    // The serial ones are live when no parallel one wrote or failed.
                    let live = live_ok
                        && !(mt_mode >= 1 && serial.is_empty())
                        && par.iter().all(|&p| {
                            let s = lock(&slots[p as usize]);
                            !stops(&s)
                                && !(s.st.effects.iter())
                                    .any(|e| matches!(e, Effect::Report(..) | Effect::Output(..)))
                        });
                    sub[2] = ts.elapsed().as_nanos() as u64;
                    for &p in &serial {
                        let mut s = lock(&slots[p as usize]);
                        if !init {
                            resumption(&self.procs, &self.known_edges, p, &mut s.st);
                        }
                        run(p, &mut s, live);
                        if stops(&s) {
                            break;
                        }
                    }
                }
                // No other thread touches the slots: no lock.
                _ => {
                    let now = self.now;
                    for (i, &p) in pids.iter().enumerate() {
                        let s = slot_mut(&mut self.slots[p as usize]);
                        if !init {
                            resumption(&self.procs, &self.known_edges, p, &mut s.st);
                        }
                        // spike(mt): one activation in 16 of each process is timed (rotating).
                        let t = (learn && (i as u64 + self.mt_probe) % 16 == 0)
                            .then(std::time::Instant::now);
                        run(p, s, live_ok);
                        if let Some(t) = t {
                            let e = &mut self.mt_cost[p as usize];
                            *e = 0.5 * *e + 0.5 * t.elapsed().as_nanos() as f32;
                        }
                        if stops(s) {
                            break;
                        }
                        // The usual clocked process, committed while its slot is hot: it waited
                        // again (`Ctx::wait_on_again`) and only scheduled drivers. The commit
                        // loop skips it (`ProcMeta::ready` is false). The order of the queue
                        // pushes across processes does not matter: the entries are sorted.
                        let st = &mut s.st;
                        let meta = &mut self.procs[p as usize];
                        if !init
                            && !postponed
                            && st.again
                            && meta.perm != 0
                            && st.sens.is_empty()
                            && st.deadline.is_none()
                            && st.effects.is_empty()
                            && st.phantoms.is_empty()
                            && s.result.is_ok()
                        {
                            st.again = false;
                            meta.ready = false;
                            meta.resumed = true;
                            self.serial_ran |= meta.flags.serial;
                            for &(t, d) in st.scheduled.iter() {
                                self.queue.push(now, t, Entry::Driver(d.0));
                            }
                            st.scheduled.clear();
                        }
                    }
                }
            }
        }
        self.stats.process_runs += pids.len() as u64;
        let run_ns = t_run.map(|t| t.elapsed().as_nanos() as u64);
        let t_commit = t_run.map(|_| std::time::Instant::now());
        // The batch is in commit order already unless it was shuffled to run.
        if self.rng.is_some() {
            self.order(&mut pids, init);
        }
        let mut first_err = None;
        for &p in &pids {
            let meta = &mut self.procs[p as usize];
            if !init && !meta.ready {
                continue; // committed right after it ran
            }
            meta.ready = false;
            meta.resumed |= !init;
            self.serial_ran |= meta.flags.serial;
            let (generation, trigger, tid) = (meta.generation, meta.trigger, meta.tid);
            let slot = slot_mut(&mut self.slots[p as usize]);
            let st = &mut slot.st;
            // The usual clocked process: it waited again on the range its resident waiter
            // already covers (`Ctx::wait_on_again`) and did nothing but schedule drivers.
            if st.again
                && !postponed
                && meta.perm != 0
                && st.sens.is_empty()
                && st.deadline.is_none()
                && st.effects.is_empty()
                && st.phantoms.is_empty()
                && slot.result.is_ok()
            {
                st.again = false;
                for &(t, d) in st.scheduled.iter() {
                    self.queue.push(self.now, t, Entry::Driver(d.0));
                }
                st.scheduled.clear();
                continue;
            }
            let effects = (!st.effects.is_empty()).then(|| std::mem::take(&mut st.effects));
            let bad = postponed
                && (st.deadline == Some(self.now)
                    || !st.phantoms.is_empty()
                    || st.scheduled.iter().any(|&(t, _)| t == self.now));
            if !st.scheduled.is_empty() {
                for &(t, d) in st.scheduled.iter() {
                    self.queue.push(self.now, t, Entry::Driver(d.0));
                }
                st.scheduled.clear();
            }
            if !st.phantoms.is_empty() {
                self.empty_delta |= st.take_phantoms();
            }
            let timed = st.deadline.is_some();
            if let Some(t) = st.deadline.take() {
                self.queue.push(self.now, t, Entry::Timeout(p, generation));
            }
            // The same wait as the last run, without a timeout: the waiters of the last run
            // stay (resident, `ProcMeta::perm`), or become so now.
            let mut again = std::mem::take(&mut st.again);
            if again && (timed || !st.sens.is_empty()) {
                // More was added to the wait (`Ctx::wait_on_again`): the last wait's ranges
                // come first, as they were first named.
                let mut all = st.sens_prev.clone();
                all.extend(st.sens.drain(..));
                st.sens = all;
                again = false;
            }
            let same = !timed && (again || !st.sens.is_empty() && st.sens[..] == st.sens_prev[..]);
            let mut perm = 0;
            if same {
                perm = self.procs[p as usize].perm;
                if perm == 0 {
                    self.perm_epoch += 1;
                    perm = self.perm_epoch;
                    self.procs[p as usize].perm = perm;
                    if again {
                        st.sens.extend_from_slice(&st.sens_prev);
                    }
                } else {
                    st.sens.clear();
                }
            } else {
                if self.procs[p as usize].perm != 0 {
                    // Its resident waiters are stale now: their signals' wake lists too.
                    for (s, _) in &st.sens_prev {
                        self.waiters[s.0 as usize].drop_cache();
                    }
                }
                self.procs[p as usize].perm = 0;
                st.sens_prev.clear();
                if !timed {
                    st.sens_prev.extend_from_slice(&st.sens);
                }
            }
            if !st.sens.is_empty() {
                for (s, range) in st.sens.drain(..) {
                    self.signals[s.0 as usize].watched = true;
                    let procs = &self.procs;
                    self.waiters[s.0 as usize].add(
                        p,
                        range,
                        Reg {
                            generation,
                            perm,
                            trigger,
                            tid,
                        },
                        |w| alive(procs, w),
                    );
                }
            }
            let result = if slot.result.is_ok() {
                Ok(())
            } else {
                std::mem::replace(&mut slot.result, Ok(()))
            };
            if bad {
                let mut e = self.err(ErrorKind::PostponedDelta);
                e.process = Some((ProcId(p), self.names[p as usize].clone()));
                first_err = Some(e);
            }
            if let Some(effects) = effects {
                self.commit(ProcId(p), effects);
            }
            if let Err(e) = result {
                first_err.get_or_insert(e);
            }
            if first_err.is_some() {
                break;
            }
        }
        if let (Some(run_ns), Some(tc)) = (run_ns, t_commit) {
            let commit_ns = tc.elapsed().as_nanos() as u64;
            let (mut par, mut ser, mut all) = (Vec::new(), 0u64, Vec::new());
            let nser = pids
                .iter()
                .filter(|&&p| self.procs[p as usize].flags.serial)
                .count();
            if per_proc {
                for &p in &pids {
                    let s = slot_mut(&mut self.slots[p as usize]);
                    let ns = std::mem::take(&mut s.ns);
                    let m = self.mt.as_mut().expect("mt");
                    m.proc_ns[p as usize] += ns;
                    m.proc_runs[p as usize] += 1;
                    all.push(ns);
                    if self.procs[p as usize].flags.serial {
                        ser += ns;
                    } else {
                        par.push(ns);
                    }
                }
            }
            let sp: u64 = par.iter().sum();
            let mx = par.iter().copied().max().unwrap_or(0);
            let g = [2, 4, 8].map(|n| greedy(&par, n, 0));
            let c = [2, 4, 8].map(|n| chunks(&par, n));
            let o = [2, 4, 8].map(|n| greedy(&par, n, ser));
            let a4 = greedy(&all, 4, 0);
            let m = self.mt.as_mut().expect("mt");
            m.line(format_args!(
                "B {} {} {} {ser} {sp} {mx} {} {} {} {} {} {} {} {} {} {run_ns} {commit_ns} {} {} {} {} {a4}\n",
                u8::from(postponed) + 2 * u8::from(init),
                pids.len(),
                nser,
                g[0], g[1], g[2], c[0], c[1], c[2], o[0], o[1], o[2],
                u8::from(pooled),
                sub[0],
                sub[1],
                sub[2]
            ));
        }
        // The previous batch's buffer becomes the next cycle's ready list: no allocation per delta.
        let mut old = std::mem::replace(&mut self.last_ran, pids);
        if self.ready.capacity() == 0 {
            old.clear();
            self.ready = old;
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Report what resolution functions and port conversions reported in the update phase, in
    /// signal order after the port conversions (so `--threads` does not matter).
    fn commit_callouts(&mut self) {
        if crate::callout::PENDING.load(Ordering::Relaxed)
            && crate::callout::PENDING.swap(false, Ordering::Relaxed)
        {
            for s in &mut self.signals {
                let ctx = format!("resolution of {}", s.name);
                (self.callouts).extend(s.callouts.drain(..).map(|(v, m)| (v, m, ctx.clone())));
            }
        }
        if self.callouts.is_empty() {
            return;
        }
        for (severity, message, context) in std::mem::take(&mut self.callouts) {
            let report = Report {
                process: None,
                context: Some(context),
                severity,
                message,
            };
            self.notify(|o, h| o.report(h, &report));
            if severity >= self.cfg.stop_severity {
                self.end.get_or_insert(EndReason::Severity(severity));
            }
        }
    }

    fn commit(&mut self, process: ProcId, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Report(severity, message, live) => {
                    let report = Report {
                        process: Some(process),
                        context: None,
                        severity,
                        message,
                    };
                    let which = if live { !self.live_mask } else { !0 };
                    self.notify_which(which, |o, h| o.report(h, &report));
                    if severity >= self.cfg.stop_severity {
                        self.end.get_or_insert(EndReason::Severity(severity));
                    }
                }
                Effect::Output(bytes, live) => {
                    let which = if live { !self.live_mask } else { !0 };
                    self.notify_which(which, |o, h| o.output(h, process, &bytes));
                }
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
        let bit = 1u16 << phase as u32;
        if self.phase_mask & bit == 0 {
            return;
        }
        let serial = if phase == Phase::NextCycle {
            std::mem::replace(&mut self.serial_ran, false)
        } else {
            true
        };
        if !serial && self.phase_always & bit == 0 && self.obs_phases.len() <= 64 {
            return;
        }
        // ponytail: a set of 64 observers; more are called on every phase they want.
        let which = (self.obs_phases.iter().take(64).enumerate()).fold(0u64, |m, (i, &(w, s))| {
            if w & bit != 0 && (serial || s & bit == 0) {
                m | 1 << i
            } else {
                m
            }
        });
        if which == 0 && self.obs_phases.len() <= 64 {
            return;
        }
        self.notify_which(which, |o, h| {
            if o.wants_phase(phase) {
                o.phase(h, phase)
            }
        });
    }

    fn notify(&mut self, f: impl FnMut(&mut dyn Observer, &mut Hook<'_>)) {
        self.notify_which(!0, f)
    }

    fn notify_which(&mut self, which: u64, mut f: impl FnMut(&mut dyn Observer, &mut Hook<'_>)) {
        let mut observers = std::mem::take(&mut self.observers);
        let mut hook = Hook {
            now: self.now,
            delta: self.delta,
            cycle: self.cycle,
            cfg: &self.cfg,
            drivers: &self.drivers,
            names: &self.names,
            queue: &mut self.queue,
            signals: &self.signals,
            ops: &mut self.ops,
            end: &mut self.end,
            wakes: &mut self.wakes,
            empty_delta: &mut self.empty_delta,
            slots: &self.slots,
            observer: 0,
        };
        for (i, o) in observers.iter_mut().enumerate() {
            if i < 64 && which >> i & 1 == 0 {
                continue;
            }
            hook.observer = i;
            f(o.as_mut(), &mut hook);
        }
        self.observers = observers;
    }
}

/// The driving and effective values of what the matured transactions of a signal made dirty.
fn finish_signal(sig: &mut Signal, now: Time, cycle: u64) -> Result<bool, ErrorKind> {
    // Quick transactions finished when they were applied.
    let fused = sig.end_apply();
    let changed = if sig.dirty.is_empty() && sig.fast_dirty.is_empty() {
        fused
    } else {
        finish_dirty(sig, now, cycle)? | fused
    };
    if fused {
        // The runs as the dirty ranges make them, whatever order the sources were applied in.
        sig.canonical_changed();
    }
    Ok(changed)
}

fn finish_dirty(sig: &mut Signal, now: Time, cycle: u64) -> Result<bool, ErrorKind> {
    if let [r] = sig.dirty.as_slice()
        && r.len == 1
        && sig.fast_dirty.is_empty()
        && let Some(changed) = sig.finish_one(r.offset, now, cycle)
    {
        sig.dirty.clear();
        return Ok(changed);
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
    // The driving values of these were written when their transactions were applied.
    let mut fast = std::mem::take(&mut sig.fast_dirty);
    for r in fast.drain(..) {
        sig.mark_active(r, now, cycle);
        changed |= sig.update_effective(r, now, cycle);
    }
    sig.fast_dirty = fast;
    Ok(changed)
}

/// Whether the process of trigger `t` must run in this cycle: its edge, or its reset level.
fn fires(signals: &[Signal], t: crate::EdgeTrigger, cycle: u64) -> bool {
    edge(signals, t, cycle)
        || t.reset
            .is_some_and(|(r, e, c)| signals[r.0 as usize].cur_code(e as usize) == c)
}

/// Whether the edge of trigger `t` holds in this cycle: an event on the element, with its
/// value and `'last_value` through `To_X01` giving the edge ('1' or 'H' after '0' or 'L' for
/// rising), or an event ending on '1' (rising) or '0' for the legacy form.
fn edge(signals: &[Signal], t: crate::EdgeTrigger, cycle: u64) -> bool {
    let s = &signals[t.signal.0 as usize];
    let i = t.elem as usize;
    if t.legacy {
        let end = if t.rising {
            StdULogic::One
        } else {
            StdULogic::Zero
        };
        return s.event_in(Range::new(t.elem, 1), cycle) && s.cur_code(i) == end as u8;
    }
    let to_x01 = |c: u8| match c {
        c if c == StdULogic::One as u8 || c == StdULogic::H as u8 => Some(true),
        c if c == StdULogic::Zero as u8 || c == StdULogic::L as u8 => Some(false),
        _ => None,
    };
    let (now, before) = (to_x01(s.cur_code(i)), to_x01(s.last_value.code(i)));
    s.event_in(Range::new(t.elem, 1), cycle) && now == Some(t.rising) && before == Some(!t.rising)
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
/// A slot nothing else can reach (the kernel thread, outside a parallel batch): no lock.
/// What the resumption of process `p` tells it, set when it runs (not when it is woken, so
/// the wake touches no slot): whether a timeout resumed it, and, for a signal that resumed a
/// process with an edge trigger, the edge that holds (`Ctx::edge`). Not for the first run.
#[inline(always)]
fn resumption(
    procs: &[ProcMeta],
    known_edges: &[Option<(SignalId, u32, bool)>],
    p: u32,
    st: &mut ProcState,
) {
    let meta = &procs[p as usize];
    st.timed_out = meta.by_timeout;
    st.known_edge = match known_edges.get((meta.tid as usize).wrapping_sub(1)) {
        Some(&k) if !meta.by_timeout => k,
        _ => None,
    };
}

fn slot_mut(m: &mut Mutex<Slot>) -> &mut Slot {
    m.get_mut().unwrap_or_else(PoisonError::into_inner)
}

fn lock(m: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn apply_op(sig: &mut Signal, op: &SignalOp, now: Time, cycle: u64) -> Result<(), ErrorKind> {
    let (off, n) = (op.range.offset as usize, op.range.len as usize);
    let before = matches!(op.mode, OpMode::Force(_) | OpMode::Release(_))
        .then(|| sig.effective_of(op.range));
    match (op.mode, &op.value) {
        (OpMode::Force(m), Some(v)) => {
            let slot = match m {
                ForceMode::Effective => &mut sig.force_eff,
                ForceMode::Driving => &mut sig.force_drv,
                ForceMode::Default => unreachable!("make_op resolves the default mode"),
            };
            let l = Signal::layer(slot, &sig.cur);
            fill_bits(&mut l.mask, off, n, true);
            l.value.copy(off, v, 0, n);
        }
        (OpMode::Release(m), _) => {
            let slot = match m {
                ForceMode::Effective => &mut sig.force_eff,
                ForceMode::Driving => &mut sig.force_drv,
                ForceMode::Default => unreachable!("make_op resolves the default mode"),
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
    if let Some(before) = before {
        let after = sig.effective_of(op.range);
        if (0..n)
            .step_by(64)
            .any(|c| before.diff_chunk(c, &after, c, (n - c).min(64)) != 0)
        {
            sig.force_ev.push(op.range);
        }
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
