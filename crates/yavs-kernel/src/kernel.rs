//! The LRM 14.7.5 simulation cycle and `SimControl`, the only API that drives simulation.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rayon::prelude::*;
use smallvec::SmallVec;

use crate::builder::{PortConv, PortMode, ProcFlags};
use crate::error::{ErrorKind, SimError};
use crate::process::{
    ACT_ASSIGNED, ACT_SLOW, Ctx, DriverInfo, Effect, LiveSink, OpMode, Out, OutB, OutV, ProcId,
    ProcState, Process, Severity, Shared, SignalOp, check_ref, make_op,
};
use crate::signal::{
    DelayMode, DriverId, ForceMode, Hits, Range, Reg, SigRef, Signal, SignalId, Waiter, Waiters,
};
use crate::time::{Time, Unit};
use crate::value::{Buf, Elem, SignalType, Value, ValueRef, fill_bits};
use yavs_logic::StdULogic;
use yavs_logic::wide::nmask;

mod hang;
pub use hang::{DeadClock, LoopCause, LoopLink, ProcWait, StuckProc, StuckReport, StuckSignal};

/// Estimated work (ns) of the parallel processes of a batch, less its largest one, from which
/// the batch goes to the thread pool; below it the hand-over costs more than it saves (#497).
const POOL_WORK_NS: u64 = 20_000;
/// One activation in this many of each process in an inline batch is timed for `SimControl::cost`.
const COST_EVERY: usize = 16;
/// One batch in this many that would go to the pool runs inline to refresh the costs.
const PROBE_EVERY: u32 = 64;
/// Pieces of a pooled batch per thread: enough to balance processes of different cost.
const PIECES_PER_THREAD: usize = 4;
/// Generators that re-arm themselves in one cycle (clocks that edge together); more resume.
const SKIP_MAX: usize = 4;
/// Batches in a row that do not pool after which held tokens go back to the token pool.
// ponytail: a count, not a time; a long serial batch after a pooled one keeps them meanwhile.
const TOKEN_IDLE: u32 = 64;

/// A shared budget of extra threads (a jobserver, `docs/parallel.md`): `take` tops the tokens
/// held up to `want` without blocking and answers how many are held; `give` returns them all.
pub trait Tokens: Send + Sync {
    fn take(&self, want: usize) -> usize;
    fn give(&self);
}

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
    /// edge true (`Builder::edge_trigger`). Off where every activation is observable: a
    /// debugger, foreign callbacks (statement coverage counts the edges only, #707).
    pub edge_triggers: bool,
    /// A zero-delay assignment that gives an unguarded, not user-resolved signal the value it
    /// already has, when its driver has nothing pending, queues no transaction (the delta
    /// still happens). Only set where nothing reads activity: the design uses neither
    /// `'active` nor `'last_active`, and no plugin or debugger sees it (coverage runs keep it: it changes no count, #707)
    /// (`docs/design/performance.md` R2b).
    pub skip_unchanged: bool,
    /// Leave a clocked process whose activation depends on nothing but signals suspended at an
    /// edge when none of them had an event since its last activation (`Builder::idle_skip`;
    /// needs `skip_unchanged` and `edge_triggers`: the skipped activation would have assigned the
    /// values it assigned before, which queues no transaction on a signal that allows skipping).
    pub idle_skip: bool,
    /// Run the activations `idle_skip` would skip anyway and panic when one does not do what
    /// the last one did (tests).
    pub idle_check: bool,
    /// A signal of more than 64 elements that processes wait on parts of lists the changes
    /// per cell (the stretch between two boundaries of the waited ranges), not per run of
    /// changed elements (`Signal::watch_part`). Value-change observers still get exact runs.
    pub cells: bool,
    /// With `threads` above 1, hand every batch of two or more processes to the pool instead of
    /// choosing by estimated work, but initialisation and postponed batches (tests: the pooled
    /// path must commit as the inline one does).
    pub pool_all: bool,
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
            idle_skip: true,
            idle_check: false,
            cells: true,
            pool_all: false,
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
    /// Processes made ready (a wake of a process that was not ready already).
    pub wakes: u64,
    /// Process runs that took the one-process path of the batch (`run_one`).
    pub single_runs: u64,
    /// Batches handed to the thread pool (`--threads` above 1, #497).
    pub pooled_batches: u64,
    /// Update phases by path: the usual delta of one independent signal (or nothing due), the
    /// single timed entry, and the general worklist. The fast cases are the point of many
    /// rounds: a design that moves to `update_general` lost one.
    pub update_fast: u64,
    pub update_timed: u64,
    pub update_general: u64,
    /// Scalar and byte transactions applied as quick transactions (no waveform).
    pub quick_applied: u64,
    /// Direct entries committed from process lists: single codes, plane vectors, byte signals.
    pub direct_codes: u64,
    pub direct_planes: u64,
    pub direct_bytes: u64,
    /// Activations of clocked processes skipped (or, with `Config::idle_check`, checked) because
    /// none of the signals they read had an event since the last one.
    pub idle_skips: u64,
    /// Signals of more than 64 elements that keep only each block's last event
    /// (`Builder::lean_events`); fixed when the model is built.
    pub lean_signals: u64,
    /// Signals that keep no byte codes (`Builder::no_codes`); fixed when the model is built.
    pub no_code_signals: u64,
    /// Signals that allow skipping an assignment of the value they hold (`Signal::skip_ok`).
    pub skip_signals: u64,
    /// Kernel signals (a pack of record leaves is one, #686).
    pub signals: u64,
    /// Assignments that took the general path of `Ctx::assign` (waveform lists, preemption);
    /// summed over the processes when asked.
    pub general_assigns: u64,
    /// Runs of concurrent copies the kernel made itself (`SimControl::forward_copies`), also
    /// counted in `process_runs`.
    pub copy_forwards: u64,
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

/// A signal range read in place (`Hook::view`).
pub struct SigView<'a> {
    buf: &'a crate::value::Buf,
    off: usize,
    len: usize,
}

impl SigView<'_> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Element `i` (0 is the first element of the range) as its `std_ulogic` code for logic
    /// storage, else its first byte.
    #[inline]
    pub fn code(&self, i: usize) -> u8 {
        self.buf.code(self.off + i)
    }

    /// Elements `first .. first + k` (`k <= 64`) of logic storage as four plane words with
    /// element `first` in bit 0 (bit `j` of word `p` is bit `p` of the code); `None` for byte
    /// storage.
    #[inline]
    pub fn planes(&self, first: usize, k: usize) -> Option<[u64; 4]> {
        match self.buf {
            crate::value::Buf::Planes { b, .. } => {
                Some(yavs_logic::wide::get_block(b, self.off + first, k))
            }
            crate::value::Buf::Bytes { .. } => None,
        }
    }

    /// The bytes of the range, for byte storage (`None` for logic storage).
    pub fn bytes(&self) -> Option<&[u8]> {
        self.buf.bytes(self.off, self.len)
    }
}

/// The process runs the simulation had done when it last started a delta cycle: how far the run
/// has got, for a reader on another thread (the tier policy compares the progress per second
/// before and after it swaps compiled code in, `yavs_interp::sim_progress`). One store per cycle,
/// by the kernel that runs; with several kernels in a process (tests) it is only a hint.
static PROGRESS: AtomicU64 = AtomicU64::new(0);

/// See [`PROGRESS`].
pub fn progress() -> u64 {
    PROGRESS.load(Ordering::Relaxed)
}

/// The interrupt flags of an observer's process context: observers run between cycles.
static NO_INTERRUPT: AtomicU8 = AtomicU8::new(0);

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
    /// The next activity time, known in the end-of-time-step phase (`Kernel::obs_next`).
    next: Option<Option<Time>>,
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

    /// The ranges of signal `s` that changed in this cycle (`Observer::value_changes`).
    pub fn changed(&self, s: SignalId) -> &[Range] {
        self.signals.get(s.0 as usize).map_or(&[], |g| &g.changed)
    }

    /// The effective value of `r` read in place: no copy, nothing allocated.
    pub fn view(&self, r: impl Into<SigRef>) -> Result<SigView<'_>, SimError> {
        let (s, r) = check_ref(self.signals, r.into()).map_err(|k| self.err(k))?;
        Ok(SigView {
            buf: &s.cur,
            off: r.offset as usize,
            len: r.len as usize,
        })
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
            interrupt: &NO_INTERRUPT,
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
        // No direct transaction: the signals are read-only here.
        ctx.spill_out(d);
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

    /// The time of the next scheduled activity (`Some(None)`: nothing is scheduled), known only
    /// to end-of-time-step observers when one says `Observer::wants_next_time` (`None` in every
    /// other call); wake-ups scheduled from the current call count.
    pub fn next_time(&self) -> Option<Option<Time>> {
        let queued = self.next?;
        let wake = self.wakes.pending.first_key_value().map(|(id, _)| id.time);
        Some(match (queued, wake) {
            (Some(q), Some(w)) => Some(q.min(w)),
            (q, w) => q.or(w),
        })
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
    /// Whether this observer asks `Hook::next_time` in the end-of-time-step phase (the default
    /// is no: the kernel then does not compute it there).
    fn wants_next_time(&self) -> bool {
        false
    }
    /// Whether `phase` matters to this observer only in a cycle after which a serial process
    /// ran (`ProcFlags::serial`, `Process::serial`): state only serial processes change, such
    /// as the shared heap. The kernel skips its `phase` call otherwise (the default is no).
    fn phase_after_serial(&self, _phase: Phase) -> bool {
        false
    }
    /// Asked right before `phase`, without a `Hook` (which costs the kernel about 50 instructions
    /// to make): false skips the call. For an observer that has something to do in a few cycles
    /// only and can tell cheaply (the default is yes).
    fn phase_due(&mut self, _phase: Phase) -> bool {
        true
    }
    fn phase(&mut self, _hook: &mut Hook<'_>, _phase: Phase) {}
    /// Elements `range` of `signal` changed value in this cycle.
    fn value_change(&mut self, _hook: &mut Hook<'_>, _signal: SignalId, _range: Range) {}

    /// All value changes of one cycle at once: the changed signals in id order, each with the
    /// ranges `Hook::changed` gives. The default calls [`Self::value_change`] for every range;
    /// an observer that costs per call (a lock, a lookup) overrides it. Observers are called
    /// one after the other, each with the whole cycle.
    fn value_changes(&mut self, hook: &mut Hook<'_>, signals: &[u32]) {
        for &s in signals {
            for k in 0..hook.changed(SignalId(s)).len() {
                let r = hook.changed(SignalId(s))[k];
                self.value_change(hook, SignalId(s), r);
            }
        }
    }
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
    /// The stop bit of the flag byte `Ctx::interrupt_flags` points at.
    pub const STOP: u8 = 2;

    pub fn pause(&self) {
        self.0.fetch_or(Self::PAUSE, Ordering::AcqRel);
    }

    pub fn stop(&self) {
        self.0.fetch_or(Self::STOP, Ordering::AcqRel);
    }

    /// Whether `stop` was called (checked inside a process that never suspends, `Ctx::stop_requested`).
    pub(crate) fn stopped(flags: &AtomicU8) -> bool {
        flags.load(Ordering::Relaxed) & Self::STOP != 0
    }
}

pub(crate) struct Slot {
    pub process: Box<dyn Process>,
    pub st: ProcState,
    pub result: Result<(), SimError>,
}

/// A piece of a pooled batch (`SimControl::run_batch`): processes of consecutive numbers from
/// `lo`, with the slices of the slots, metas and idle flags from `lo` on.
struct Piece<'a> {
    pids: &'a [u32],
    lo: usize,
    slots: &'a mut [Mutex<Slot>],
    procs: &'a mut [ProcMeta],
    armed: &'a mut [bool],
    out: Early,
}

/// The estimated time of a pooled batch inline (`all`, ns) and of its splittable part (`work`);
/// `learn`: every process has a cost (`SimControl::learn_pooled`).
#[derive(Clone, Copy, Default)]
struct Estimate {
    all: u64,
    work: u64,
    learn: bool,
}

/// What the processes of a piece committed in the worker, for the kernel thread to apply, and
/// the processes it left to the commit loop (`late`, in process order).
#[derive(Default)]
struct Early {
    queue: Vec<(Time, u32)>,
    direct: Vec<Out>,
    directv: Vec<OutV>,
    directb: Vec<OutB>,
    empty_delta: bool,
    idle_done: u32,
    late: Vec<u32>,
    /// Serial processes the worker left for the kernel thread.
    serial: u32,
    /// A process of the piece failed.
    failed: bool,
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
    /// The edge (`edge_key`) of the dynamic trigger the process's waiters were registered with
    /// (`Ctx::wait_edge`), `NO_EDGE` for none (always, with a `trigger`).
    pub wkey: u64,
    /// An activation can be skipped when `armed` is false (`Builder::idle_skip`, set by
    /// `SimControl::new` for the processes that qualify).
    pub idle_ok: bool,
    /// Tracked activations of an `idle_ok` process in the current window (`IDLE_WINDOW`) and how
    /// many of them were skipped: with too few skips it is off (`idle_ok` false) until
    /// `SimControl::idle_reprobe`.
    pub idle_win: u16,
    pub idle_hits: u16,
    /// What `idle_ok` was set to at the start (`idle_reprobe` restores it).
    pub idle_static: bool,
    /// The last activation: 0 unknown (or not a plain clocked activation), 1 assigned nothing,
    /// 2 assigned.
    pub idle_state: u8,
    /// `Config::idle_check`: this activation would have been skipped.
    pub shadow: bool,
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
    later: TimeHeap,
    /// Entries at the current time, in push order.
    next_delta: Vec<Entry>,
}

/// A heap item: `Entry` flattened to words. Ordered by `(time, key)`: the key puts drivers
/// before timeouts before implicit signals, then by index, as `Entry` does; `generation` is the
/// generation of a timeout (which of two timeouts of one process at one time is live does not
/// matter to the order).
#[derive(Clone, Copy)]
struct Later {
    time: Time,
    key: u64,
    generation: u64,
}

impl Later {
    fn entry(self) -> Entry {
        let id = self.key as u32;
        match self.key >> 32 {
            0 => Entry::Driver(id),
            1 => Entry::Timeout(id, self.generation),
            _ => Entry::Implicit(id),
        }
    }
}

/// A binary min-heap of `Later`, written out so a push keeps the new item in registers and
/// writes it field by field: the generic heap took it by pointer and copied it as one wide
/// load of fields just stored narrower, which cost a store-forwarding stall per push.
#[derive(Default)]
struct TimeHeap {
    v: Vec<Later>,
}

impl TimeHeap {
    #[inline(always)]
    fn push(&mut self, time: Time, key: u64, generation: u64) {
        let mut i = self.v.len();
        self.v.push(Later {
            time,
            key,
            generation,
        });
        while i > 0 {
            let parent = (i - 1) / 2;
            let p = self.v[parent];
            if (p.time, p.key) <= (time, key) {
                break;
            }
            self.v[i] = p;
            i = parent;
        }
        self.v[i] = Later {
            time,
            key,
            generation,
        };
    }

    #[inline(always)]
    fn peek(&self) -> Option<&Later> {
        self.v.first()
    }

    /// The time of the second earliest item (the smaller of the root's children).
    #[inline(always)]
    fn second_time(&self) -> Option<Time> {
        match (self.v.get(1), self.v.get(2)) {
            (Some(a), Some(b)) => Some(a.time.min(b.time)),
            (Some(a), None) => Some(a.time),
            _ => None,
        }
    }

    fn pop(&mut self) {
        let Some(last) = self.v.pop() else { return };
        let n = self.v.len();
        if n == 0 {
            return;
        }
        let (time, key) = (last.time, last.key);
        let mut i = 0;
        loop {
            let mut c = 2 * i + 1;
            if c >= n {
                break;
            }
            if c + 1 < n
                && (self.v[c + 1].time, self.v[c + 1].key) < (self.v[c].time, self.v[c].key)
            {
                c += 1;
            }
            if (self.v[c].time, self.v[c].key) >= (time, key) {
                break;
            }
            self.v[i] = self.v[c];
            i = c;
        }
        self.v[i] = last;
    }
}

impl Queue {
    #[inline(always)]
    fn push(&mut self, now: Time, t: Time, e: Entry) {
        if t == now {
            self.next_delta.push(e);
        } else {
            match e {
                Entry::Driver(d) => self.later.push(t, u64::from(d), 0),
                Entry::Timeout(p, g) => self.later.push(t, 1 << 32 | u64::from(p), g),
                Entry::Implicit(i) => self.later.push(t, 2 << 32 | u64::from(i), 0),
            }
        }
    }

    /// The earliest future entry.
    #[inline(always)]
    fn peek(&self) -> Option<(Time, Entry)> {
        self.later.peek().map(|l| (l.time, l.entry()))
    }

    /// The time of the earliest future entry.
    #[inline(always)]
    fn peek_time(&self) -> Option<Time> {
        self.later.peek().map(|l| l.time)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Entry {
    Driver(u32),
    Timeout(u32, u64),
    Implicit(u32),
}

/// A clock generator: `x <= not x after T` as `ToggleProcess` runs it.
struct Toggle {
    driver: u32,
    /// `T` in the simulation resolution; positive.
    period: Time,
    proc: u32,
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
    /// The sources of the implicit signals (signal, range), each once: an update in which none
    /// of them was active, with no implicit signal due, has nothing for `update_implicit`.
    implicit_srcs: Vec<(u32, Range)>,
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
    /// Direct transactions of processes committed in the batch loop (`run_batch`), for the
    /// signals once the batch is over.
    direct: Vec<Out>,
    directv: Vec<OutV>,
    directb: Vec<OutB>,
    /// Signals with direct transactions (`commit_out`) for the next update.
    direct_sigs: Vec<u32>,
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
    /// The processes of a clock that run as one batch in this cycle (`batch_ok`).
    clock_batch: Option<Vec<u32>>,
    /// The batch `run_batch` is about to run is known to be in order.
    batch_mono: bool,
    /// Clock generators (`Process::generator`), and per driver the index of its entry (or
    /// `u32::MAX`; empty without generators).
    toggles: Vec<Toggle>,
    toggle_ix: Vec<u32>,
    /// The generators whose resumption this cycle's events make unnecessary (`rearm`): the first
    /// `skip_n` of `skip_wake`.
    skip_wake: [u32; SKIP_MAX],
    skip_n: usize,
    /// The `_guard` processes of guarded blocks (`Builder::guard_process`) and, per signal, the
    /// ones whose conditions read it. Their `ProcMeta::ready` stays true, so no wake queues them.
    guard_procs: Vec<u32>,
    guard_readers: HashMap<u32, SmallVec<[u32; 2]>>,
    guard_due: Vec<u32>,
    /// Per signal, the processes of `idle_ok` that read it: `readers[readers_start[s]..
    /// readers_start[s + 1]]` (empty without such a process).
    readers_start: Vec<u32>,
    /// Per process: a signal it reads had an event since its last activation (`Builder::idle_skip`;
    /// a dense array, the readers' loop of every event touches one byte per reader).
    armed: Vec<bool>,
    readers: Vec<Reader>,
    reader_guards: Vec<ReaderGuard>,
    idle_active: bool,
    /// Processes with `idle_ok` now: `idle_active` is off when none is left, until `idle_reprobe`.
    idle_live: u32,
    /// Some process qualified for the idle skip (`idle_reprobe` has work).
    idle_live_possible: bool,
    /// `Tn` as the last delta computed it (only a cycle sets it, only `run_until` takes it).
    known_next: Option<Option<Time>>,
    /// The next activity time while end-of-time-step observers run (`Hook::next_time`).
    obs_next: Option<Option<Time>>,
    /// Some observer wants `obs_next` (`Observer::wants_next_time`).
    next_wanted: bool,
    postponed: Vec<u32>,
    /// Per process, the parts of a concurrent copy the kernel commits itself
    /// (`forward_copies`); empty when the design has none.
    fwd: Vec<Option<Box<[Fwd]>>>,
    /// The blocks of a wide copy part, kept between runs.
    fwd_run: Vec<OutV>,
    last_ran: Vec<u32>,
    /// The commit list of `run_batch`, kept between batches.
    late: Vec<u32>,
    /// The sort keys of `order`, kept between batches.
    order_keys: Vec<u64>,
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
    /// The token pool that decides how many of the pool's workers may run (`set_tokens`), the
    /// tokens held and the batches in a row that did not pool since.
    tokens: Option<std::sync::Arc<dyn Tokens>>,
    held: usize,
    token_idle: u32,
    /// With a pool: per process the estimated time of an activation in ns, an average of timed
    /// inline activations (0 until one is timed); the count of batches, which places the timed
    /// activations, and of the batches above and below `pool_ns`, which pick the probes (#497).
    cost: Vec<u32>,
    /// With a pool: per process, whether it is serial (dense, written only at build: the metas
    /// are written by the workers).
    serial_of: Vec<bool>,
    /// The buffers of the pieces of a pooled batch, kept between batches.
    early: Vec<Early>,
    /// The threshold of `pool_batch` in ns: `POOL_WORK_NS`, raised by pooled batches that lost.
    pool_ns: u64,
    /// `learn_pooled`: pooled time over the estimate inline, in 1024ths, averaged.
    pool_ratio: u64,
    batches: u32,
    probe: u32,
    raise_probe: u32,
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
    /// `triggers` by trigger: the elaborator's and the ones waits made (`Ctx::wait_edge`).
    trig_ids: std::collections::HashMap<crate::EdgeTrigger, u32>,
    /// Named parts of signals (record packs) for hang and loop reports (`name_parts`).
    part_names: HashMap<u32, Vec<(Range, String)>>,
}

/// Signal `a` to read and signal `b` to write (distinct).
fn signal_pair(v: &mut [Signal], a: usize, b: usize) -> (&Signal, &mut Signal) {
    if a < b {
        let (x, y) = v.split_at_mut(b);
        (&x[a], &mut y[0])
    } else {
        let (x, y) = v.split_at_mut(a);
        (&y[0], &mut x[b])
    }
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
        mut slots: Vec<Mutex<Slot>>,
        mut procs: Vec<ProcMeta>,
        names: Vec<String>,
        idles: Vec<Option<Vec<crate::builder::IdleRead>>>,
        guards: Vec<Option<Vec<(SignalId, Range)>>>,
    ) -> Result<SimControl, SimError> {
        if let Some(pr) = crate::profile::installed() {
            for (s, name) in slots.iter_mut().zip(&names) {
                let s = slot_mut(s);
                let inner = std::mem::replace(&mut s.process, Box::new(|_: &mut Ctx<'_>| Ok(())));
                s.process = pr.wrap(name, inner);
            }
        }
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
            // The kernel thread works too: N threads in all, so a pinned run is not oversubscribed.
            n => Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n - 1)
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
        let mut implicit_srcs: Vec<(u32, Range)> = Vec::new();
        for im in &implicits {
            implicit_srcs.push((im.src.0, im.range));
            implicit_srcs.extend(im.more.iter().map(|&(s, r)| (s.0, r)));
        }
        implicit_srcs.sort_by_key(|&(s, r)| (s, r.offset, r.len));
        implicit_srcs.dedup();
        if cfg.skip_unchanged {
            for s in &mut signals {
                s.inactive = true;
            }
            for im in &implicits {
                signals[im.src.0 as usize].inactive = false;
                for &(s, _) in &im.more {
                    signals[s.0 as usize].inactive = false;
                }
            }
        }
        for s in &mut signals {
            s.set_static_plain();
        }
        let mut drivers = drivers;
        for d in &mut drivers {
            let s = &signals[d.signal.0 as usize];
            d.direct = independent[d.signal.0 as usize]
                && matches!(s.ty.elem, Elem::Logic)
                && s.guard.is_none()
                && matches!(
                    s.resolution,
                    None | Some(crate::signal::Resolution::StdLogic)
                )
                && !s.drivers[d.slot as usize].shared;
            d.bdirect = independent[d.signal.0 as usize]
                && matches!(s.ty.elem, Elem::Bytes(es) if es <= 8)
                && s.guard.is_none()
                && s.resolution.is_none()
                && !s.drivers[d.slot as usize].shared;
        }
        let fwd = copy_forwards(&mut slots, &procs, &signals, &drivers);
        // Clocked processes that can be left suspended when none of the signals they read had an
        // event: pure ones (`Builder::idle_skip`) that drive signals allowing skipped assignments.
        let mut idle_active = false;
        let mut idle_live = 0u32;
        let mut readers_of: Vec<Vec<(u32, Option<Alts>, Range)>> = Vec::new();
        if cfg.idle_skip && cfg.skip_unchanged && cfg.edge_triggers {
            let mut bad = vec![false; procs.len()];
            for d in &drivers {
                bad[d.owner.0 as usize] |= !signals[d.signal.0 as usize].skip_ok;
            }
            for (p, reads) in idles.iter().enumerate() {
                let Some(reads) = reads else { continue };
                let m = &mut procs[p];
                if bad[p]
                    || m.flags.postponed
                    || m.flags.several_waits
                    || m.flags.serial
                    || m.flags.concurrent_call
                {
                    continue;
                }
                if readers_of.is_empty() {
                    readers_of = vec![Vec::new(); signals.len()];
                }
                m.idle_ok = true;
                m.idle_static = true;
                idle_active = true;
                idle_live += 1;
                for r in reads {
                    let l = &mut readers_of[r.signal.0 as usize];
                    match l.last_mut() {
                        // Read twice (two parts of the signal, or under two conditions): the
                        // read happens when either condition holds.
                        Some((q, g, h)) if *q == p as u32 => {
                            let end = (h.offset + h.len).max(r.range.offset + r.range.len);
                            h.offset = h.offset.min(r.range.offset);
                            h.len = end - h.offset;
                            *g = match (g.take(), &r.guard) {
                                (Some(mut a), Some(b)) => {
                                    a.extend(b.iter().cloned());
                                    Some(a)
                                }
                                _ => None,
                            }
                        }
                        _ => l.push((p as u32, r.guard.clone(), r.range)),
                    }
                }
            }
        }
        let mut readers_start = Vec::new();
        let mut readers = Vec::new();
        let mut reader_guards = Vec::new();
        if idle_active {
            for l in readers_of {
                readers_start.push(readers.len() as u32);
                for (p, g, r) in l {
                    // A read of part of the signal is armed by its changed runs: list them.
                    let sig = &mut signals[readers_start.len() - 1];
                    let whole = r.offset == 0 && r.len >= sig.ty.len;
                    if r != Range::new(0, sig.ty.len) {
                        sig.watched = true;
                        sig.watch_part(r, cfg.cells);
                    }
                    let g = g.map_or(NO_GUARD, |g| {
                        reader_guards.push(match &g[..] {
                            [c] if c.len() == 1 => ReaderGuard::One(c[0]),
                            _ => ReaderGuard::Any(g),
                        });
                        reader_guards.len() as u32 - 1
                    });
                    let mask = if sig.ty.len <= 64 && !whole {
                        nmask(r.len as usize) << r.offset
                    } else {
                        0
                    };
                    let r = if whole { Range::new(0, u32::MAX) } else { r };
                    readers.push(Reader { p, g, r, mask });
                }
            }
            readers_start.push(readers.len() as u32);
        }
        // The guard processes of guarded blocks, by the signals their conditions read.
        let mut guard_procs = Vec::new();
        let mut guard_readers: HashMap<u32, SmallVec<[u32; 2]>> = HashMap::new();
        for (p, reads) in guards.iter().enumerate() {
            let Some(reads) = reads else { continue };
            guard_procs.push(p as u32);
            // Never woken by its waiters: it runs in `run_guards`.
            procs[p].ready = true;
            for (s, _) in reads {
                let l = guard_readers.entry(s.0).or_default();
                if !l.contains(&(p as u32)) {
                    l.push(p as u32);
                }
            }
        }
        let lean_signals = signals.iter().filter(|s| s.is_lean()).count() as u64;
        let no_code_signals = signals.iter().filter(|s| s.keeps_no_codes()).count() as u64;
        let skip_signals = signals.iter().filter(|s| s.skip_ok).count() as u64;
        let nsignals = signals.len() as u64;
        let (cost, serial_of) = if pool.is_some() {
            (
                vec![0; procs.len()],
                procs.iter().map(|m| m.flags.serial).collect(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let mut sim = SimControl {
            guard_procs,
            guard_readers,
            guard_due: Vec::new(),
            readers_start,
            armed: vec![true; procs.len()],
            readers,
            reader_guards,
            idle_active,
            idle_live,
            idle_live_possible: idle_live != 0,
            names,
            rng: cfg.shuffle,
            cfg,
            waiters: signals.iter().map(|s| Waiters::new(s.ty.len)).collect(),
            known_next: None,
            obs_next: None,
            next_wanted: false,
            work: None,
            signals,
            drivers,
            ports,
            ports_up,
            ports_down,
            independent,
            implicits,
            implicit_srcs,
            group_cycle: vec![0; groups.len()],
            groups,
            slots,
            single_waits: procs.iter().all(|m| !m.flags.several_waits),
            procs,
            queue: Queue::default(),
            direct: Vec::new(),
            directv: Vec::new(),
            directb: Vec::new(),
            direct_sigs: Vec::new(),
            ops: Vec::new(),
            changed_sigs: Vec::new(),
            ready: Vec::new(),
            ready_mono: true,
            ready_several: Vec::new(),
            clock_batch: None,
            batch_mono: false,
            toggles: Vec::new(),
            toggle_ix: Vec::new(),
            skip_wake: [0; SKIP_MAX],
            skip_n: 0,
            postponed: Vec::new(),
            fwd,
            fwd_run: Vec::new(),
            last_ran: Vec::new(),
            late: Vec::new(),
            order_keys: Vec::new(),
            now: Time::ZERO,
            delta: 0,
            cycle: 1,
            started: false,
            end: None,
            end_fired: false,
            failed: None,
            observers: Vec::new(),
            live_sink: None,
            part_names: HashMap::new(),
            live_mask: 0,
            phase_mask: 0,
            phase_always: 0,
            obs_phases: Vec::new(),
            serial_ran: true,
            watch_changes: false,
            wakes: Wakes::default(),
            cost,
            serial_of,
            early: Vec::new(),
            pool,
            tokens: None,
            held: 0,
            token_idle: 0,
            pool_ns: POOL_WORK_NS,
            pool_ratio: 512,
            batches: 0,
            probe: 0,
            raise_probe: 0,
            interrupt: Interrupt::default(),
            stats: Stats {
                lean_signals,
                no_code_signals,
                skip_signals,
                signals: nsignals,
                ..Stats::default()
            },
            live: false,
            callouts: Vec::new(),
            empty_delta: false,
            perm_epoch: 0,
            woken_buf: Vec::new(),
            indep_buf: Vec::new(),
            triggers,
            trig_ids: ids,
        };
        for (m, slot) in sim.procs.iter().zip(&mut sim.slots) {
            if let Some(t) = m.trigger {
                let st = &mut slot_mut(slot).st;
                if cfg_edge_triggers && t.reset.is_none() && t.elem < 1 << 30 {
                    let kind = u8::from(t.rising) | if t.legacy { crate::EDGE_LEVEL } else { 0 };
                    st.known_edge = Some((t.signal, t.elem, kind));
                    st.known_key = crate::process::edge_key(t.signal, t.elem, kind);
                }
            }
        }
        if cfg_edge_triggers {
            for p in 0..sim.slots.len() {
                let g = slot_mut(&mut sim.slots[p]).process.generator();
                if let Some((d, t)) = g {
                    let period = t.quantize(sim.cfg.resolution);
                    if period == Time::ZERO || sim.drivers[d.0 as usize].owner.0 as usize != p {
                        continue;
                    }
                    if sim.toggle_ix.is_empty() {
                        sim.toggle_ix = vec![u32::MAX; sim.drivers.len()];
                    }
                    sim.toggle_ix[d.0 as usize] = sim.toggles.len() as u32;
                    sim.toggles.push(Toggle {
                        driver: d.0,
                        period,
                        proc: p as u32,
                    });
                }
            }
        }
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
    fn push_down(&mut self, s: u32, active: bool) -> Result<SmallVec<[u32; 4]>, ErrorKind> {
        let mut touched = SmallVec::new();
        for &e in &self.ports_down[s as usize] {
            let p = &self.ports[e as usize];
            let is_active =
                |(a, r): &(SignalId, Range)| self.signals[a.0 as usize].active_in(*r, self.cycle);
            // The usual port: one actual, one formal, no conversion. The value is copied from
            // the actual's storage into the formal's, with no buffer in between.
            if p.conv.to_formal.is_none()
                && let ([(a, ar)], [(f, fr)]) = (&p.actual[..], &p.formal[..])
                && a != f
            {
                if active && !is_active(&(*a, *ar)) {
                    continue;
                }
                let idle = match active && fr.len == ar.len {
                    true => self.signals[a.0 as usize].idle_elems(*ar, self.cycle),
                    false => None,
                };
                let (src, formal) = signal_pair(&mut self.signals, a.0 as usize, f.0 as usize);
                let layer = formal.port_in.as_mut().ok_or_else(|| {
                    ErrorKind::Internal(format!("in-port {} without a port layer", formal.name))
                })?;
                layer.value.copy(
                    fr.offset as usize,
                    &src.cur,
                    ar.offset as usize,
                    fr.len as usize,
                );
                if active {
                    match idle {
                        Some(idle) => formal.mark_active_except(*fr, &idle, self.now, self.cycle),
                        None => formal.mark_active(*fr, self.now, self.cycle),
                    }
                    formal.eff_dirty.push(*fr);
                }
                touched.push(f.0);
                continue;
            }
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
        Stats {
            general_assigns: self.slots.iter().map(|s| lock(s).st.general).sum(),
            ..self.stats
        }
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

    /// Effective value read in place (see `Hook::view`).
    pub fn view(&self, r: impl Into<SigRef>) -> Result<SigView<'_>, SimError> {
        let (s, r) = self.check(r)?;
        Ok(SigView {
            buf: &s.cur,
            off: r.offset as usize,
            len: r.len as usize,
        })
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
        self.next_wanted = self.observers.iter().any(|o| o.wants_next_time());
        if self.watch_changes {
            self.signals.iter_mut().for_each(|s| {
                s.watched = true;
                s.fine = true;
                s.cuts = Box::default();
            });
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

    /// Run the pool's workers only on tokens taken from `t`: a pooled step uses one worker per
    /// token held, and none without one (`docs/parallel.md` "Thread budget").
    pub fn set_tokens(&mut self, t: std::sync::Arc<dyn Tokens>) {
        self.tokens = Some(t);
    }

    /// The workers a pooled step may use: all of them without a token pool, else the tokens
    /// held, topped up without blocking.
    fn helpers(&mut self) -> usize {
        let n = self.pool.as_ref().map_or(0, |p| p.current_num_threads());
        let Some(t) = &self.tokens else { return n };
        self.token_idle = 0;
        if self.held < n {
            self.held = t.take(n);
        }
        self.held
    }

    /// A batch that did not pool: after `TOKEN_IDLE` in a row the tokens go back.
    #[cold]
    fn tokens_idle(&mut self) {
        self.token_idle += 1;
        if self.token_idle >= TOKEN_IDLE {
            self.give_tokens();
        }
    }

    fn give_tokens(&mut self) {
        if let Some(t) = &self.tokens
            && self.held != 0
        {
            t.give();
            self.held = 0;
        }
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
                for &g in &self.guard_procs {
                    self.procs[g as usize].ready = true;
                }
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
        // Between runs no batch runs: the tokens go back.
        self.give_tokens();
        r
    }

    /// Ended (firing `EndOfSim` once) or a pause request.
    #[inline(always)]
    fn poll_end(&mut self) -> Result<Option<RunStatus>, SimError> {
        // A plain load per delta; the rest only when something was requested or the end came.
        if self.end.is_none() && self.interrupt.0.load(Ordering::Acquire) == 0 {
            return Ok(None);
        }
        self.poll_end_slow()
    }

    #[inline(never)]
    fn poll_end_slow(&mut self) -> Result<Option<RunStatus>, SimError> {
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
        if !self.ops.is_empty() || self.empty_delta || !self.direct_sigs.is_empty() {
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

    /// The `_guard` processes whose conditions read a signal with an event in this update run now,
    /// before anything is woken: GUARD changes in the cycle of the event (LRM 11.2, 14.7.5.3), and
    /// so do the processes that wait for it. A guard whose condition reads another GUARD runs
    /// again after that one.
    fn run_guards(&mut self) -> Result<(), SimError> {
        let mut seen = 0;
        loop {
            let mut due = std::mem::take(&mut self.guard_due);
            for i in seen..self.changed_sigs.len() {
                if let Some(l) = self.guard_readers.get(&self.changed_sigs[i]) {
                    for &g in l {
                        if !due.contains(&g) {
                            due.push(g);
                        }
                    }
                }
            }
            seen = self.changed_sigs.len();
            if due.is_empty() {
                self.guard_due = due;
                return Ok(());
            }
            due.sort_unstable();
            self.run_batch(due.clone(), false, false)?;
            self.finish_direct().map_err(|k| self.err(k))?;
            for &g in &due {
                self.procs[g as usize].ready = true;
            }
            due.clear();
            self.guard_due = due;
        }
    }

    /// Signal `s` had an event: the clocked processes that read it must run at their next edge.
    #[inline(always)]
    fn arm_readers(&mut self, s: u32) {
        if self.readers_start[s as usize] != self.readers_start[s as usize + 1] {
            self.arm_readers_of(s);
        }
    }

    #[inline(always)]
    fn arm_readers_of(&mut self, s: u32) {
        let (a, b) = (
            self.readers_start[s as usize] as usize,
            self.readers_start[s as usize + 1] as usize,
        );
        let sig = &self.signals[s as usize];
        let changed = &sig.changed;
        // A signal whose changed runs are not listed arms every reader.
        let any = changed.is_empty();
        // The elements of a signal of at most 64 that changed, when a reader of a part asks.
        let mut bits: Option<u64> = None;
        for rd in &self.readers[a..b] {
            if self.armed[rd.p as usize] {
                continue;
            }
            // A read of part of the signal (a field of a record pack, #686) arms only for an
            // event in it.
            let hit = rd.r.len == u32::MAX
                || any
                || if rd.mask != 0 {
                    *bits.get_or_insert_with(|| {
                        changed
                            .iter()
                            .fold(0, |m, c| m | nmask(c.len as usize) << c.offset)
                    }) & rd.mask
                        != 0
                } else {
                    changed.iter().any(|c| c.overlaps(rd.r))
                };
            if hit && (rd.g == NO_GUARD || self.reader_guards[rd.g as usize].holds(&self.signals)) {
                self.armed[rd.p as usize] = true;
            }
        }
    }

    /// Take the clocked processes out of `pids` that need not run at this edge: none of the
    /// signals they read had an event since their last activation, so it would do what the
    /// last one did (`Builder::idle_skip`). That one assigned values (`idle_state` 2) that
    /// queue nothing on a signal that allows skipping, but they make a delta cycle, as the
    /// skipped activation would. With `Config::idle_check` the process runs and its activation
    /// is checked (`check_idle`).
    fn idle_filter(&mut self, pids: &mut Vec<u32>) {
        let mut keep = 0;
        for i in 0..pids.len() {
            let p = pids[i];
            let m = &mut self.procs[p as usize];
            if m.idle_ok && !self.armed[p as usize] && m.idle_state != 0 && m.perm != 0 {
                self.stats.idle_skips += 1;
                m.idle_hits = m.idle_hits.saturating_add(1);
                m.idle_win = m.idle_win.saturating_add(1);
                if self.cfg.idle_check {
                    m.shadow = true;
                } else {
                    m.ready = false;
                    self.empty_delta |= m.idle_state == 2;
                    continue;
                }
            }
            pids[keep] = p;
            keep += 1;
        }
        pids.truncate(keep);
    }

    /// Track again the processes `note_idle` stopped tracking: their inputs may be quiet now.
    fn idle_reprobe(&mut self) {
        for (m, armed) in self.procs.iter_mut().zip(&mut self.armed) {
            if m.idle_static && !m.idle_ok {
                m.idle_ok = true;
                m.idle_win = 0;
                m.idle_hits = 0;
                *armed = true;
                self.idle_live += 1;
            }
        }
        self.idle_active = self.idle_live != 0;
    }

    fn next_queued(&mut self) -> Option<Time> {
        while let Some(&e) = self.queue.next_delta.last() {
            if self.live(self.now, e) {
                return Some(self.now);
            }
            self.queue.next_delta.pop();
        }
        while let Some((t, e)) = self.queue.peek() {
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
                let cause = self.loop_cause();
                return Err(self.err(ErrorKind::DeltaLimit {
                    limit: self.cfg.max_deltas,
                    processes,
                    cause,
                }));
            }
            self.delta += 1;
        }
        self.cycle += 1;
        self.stats.cycles += 1;
        PROGRESS.store(self.stats.process_runs, Ordering::Relaxed);
        if self.idle_live_possible && self.cycle & 0xffff == 0 {
            self.idle_reprobe();
        }
        self.phase(Phase::NextCycle);
        if !self.wakes.pending.is_empty() {
            self.fire_wakes();
        }
        self.update().map_err(|k| self.err(k))?;
        self.commit_callouts();
        if !self.guard_procs.is_empty() {
            self.run_guards()?;
        }
        if self.changed_sigs.len() > 1 {
            self.changed_sigs.sort_unstable();
            self.changed_sigs.dedup();
        }

        // Value-change bus, then 14.7.5.3 d: resume sensitive processes.
        if self.watch_changes {
            self.notify_changes();
        }
        let memo = std::cell::Cell::new((0u32, false));
        // Nothing below adds to `changed_sigs`: walked in place, cleared after.
        for i in 0..self.changed_sigs.len() {
            let s = self.changed_sigs[i];
            if self.idle_active {
                self.arm_readers(s);
            }
            let sig = &mut self.signals[s as usize];
            self.stats.events += sig.changed.len() as u64 + std::mem::take(&mut sig.nruns);
            // No process ever waited for it (a signal only read by value).
            if self.waiters[s as usize].is_unwatched() {
                sig.changed.clear();
                continue;
            }
            // Only the clock generator that re-armed itself waits for it: nothing to wake.
            if self.skip_n != 0
                && self.skip_wake[..self.skip_n]
                    .iter()
                    .any(|&p| self.waiters[s as usize].only_resident(p))
            {
                sig.changed.clear();
                continue;
            }
            // Update order is element order, mostly: sort only when it is not. Cells of one
            // update overlap the runs of another: join them.
            if !sig.cuts.is_empty() {
                sig.canonical_changed();
            } else if !sig.changed.is_sorted_by_key(|r| r.offset) {
                sig.changed.sort_unstable_by_key(|r| r.offset);
            }
            let waiters = &mut self.waiters[s as usize];
            let signals = &self.signals;
            let runs = &signals[s as usize].changed;
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
            // One element changed and one resident process waits for it (a stage of a chain):
            // no list scan.
            if let [r] = runs[..]
                && r.len == 1
                && let Some(w) = waiters.sole_resident(r.offset)
                && alive(&self.procs, w)
            {
                let (p, held) = (w.proc, uniform.is_none() && hold(w));
                self.signals[s as usize].changed.clear();
                if !held {
                    self.wake(p, false);
                }
                continue;
            }
            // Events on a signal with a few waiters, those they hit resident and alive: the same wake
            // as the scan below makes, in list order, with nothing to remove from the list. A waiter
            // that is not (a plain one, a stale one) leaves the rest to the scan: waking is idempotent
            // and in list order, so the scan wakes the others in the same order.
            if !runs.is_empty()
                && let Some(list) = waiters.small_list(8)
            {
                let mut plain = true;
                let hits = Hits::of(signals[s as usize].ty.len, runs);
                for w in list {
                    if !hits.hit(w) {
                        continue;
                    }
                    if w.perm == 0 || !alive(&self.procs, w) {
                        plain = false;
                        break;
                    }
                    if !(uniform.is_none() && hold(w)) {
                        wake_proc(
                            &mut self.procs,
                            &mut self.postponed,
                            &mut self.ready_several,
                            &mut self.ready_mono,
                            &mut self.ready,
                            &mut self.stats.wakes,
                            w.proc,
                            false,
                        );
                    }
                }
                if plain {
                    self.signals[s as usize].changed.clear();
                    continue;
                }
            }
            // Every waiter resident for the whole signal: the wake list is known.
            if let Some(tids) = waiters.cache_tids() {
                let held: smallvec::SmallVec<[(u32, bool); 8]> = tids
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
                let is_held = |t: u32| held.iter().any(|&(u, h)| u == t && h);
                let batch = self.batch_of(s as usize, &list);
                let waiters = std::mem::take(&mut self.waiters[s as usize]);
                match waiters.cache_batch() {
                    // The clock's plain processes, all resident, run as one batch in list
                    // order, those of the triggers that hold: no wake per process (a resident
                    // waiter needs no new generation, and nothing else wakes a process in this
                    // cycle). The others wake as usual and run after them.
                    Some(Some(b))
                        if batch
                            && self.changed_sigs.len() == 1
                            && self.ready.is_empty()
                            && self.ready_several.is_empty()
                            && self.rng.is_none() =>
                    {
                        let mut pids = std::mem::take(&mut self.ready);
                        if b.by_tid.iter().all(|&(t, _)| !is_held(t)) {
                            pids.extend_from_slice(&b.plain);
                        } else {
                            let mut runs = 0;
                            for (t, l) in &b.by_tid {
                                if !is_held(*t) && !l.is_empty() {
                                    pids.extend_from_slice(l);
                                    runs += 1;
                                }
                            }
                            if runs > 1 {
                                pids.sort();
                            }
                        }
                        for &(p, tid) in &b.others {
                            if tid == 0 || !is_held(tid) {
                                self.wake(p, false);
                            }
                        }
                        for &p in &self.skip_wake[..self.skip_n] {
                            // `pids` is in process order: find it, or fall back to a scan.
                            match pids.binary_search(&p) {
                                Ok(i) => _ = pids.remove(i),
                                Err(_) => pids.retain(|&q| q != p),
                            }
                        }
                        if !pids.is_empty() {
                            self.clock_batch = Some(pids);
                        } else {
                            self.ready = pids;
                        }
                    }
                    _ => {
                        for &(_, p, tid) in &list {
                            if tid == 0 || !is_held(tid) {
                                self.wake(p, false);
                            }
                        }
                    }
                }
                self.waiters[s as usize] = waiters;
                self.waiters[s as usize].restore_cache(list);
                continue;
            }
            let mut woken = std::mem::take(&mut self.woken_buf);
            // A shuffled run takes the order the processes are woken in.
            let ordered = self.rng.is_some();
            match uniform {
                Some(_) => waiters.take_woken(
                    runs,
                    |w| alive(&self.procs, w),
                    |_| false,
                    &mut woken,
                    ordered,
                ),
                None => {
                    waiters.take_woken(runs, |w| alive(&self.procs, w), hold, &mut woken, ordered)
                }
            }
            self.signals[s as usize].changed.clear();
            for &(_, p) in &woken {
                self.wake(p, false);
            }
            self.woken_buf = woken;
        }

        self.changed_sigs.clear();
        if self.skip_n != 0 {
            for &p in &self.skip_wake[..self.skip_n] {
                self.procs[p as usize].ready = false;
            }
            self.skip_n = 0;
        }

        // e: run the resumed non-postponed processes.
        let mut ready = std::mem::take(&mut self.ready);
        self.batch_mono = std::mem::replace(&mut self.ready_mono, true);
        if let Some(pids) = self.clock_batch.take() {
            ready = pids;
            self.batch_mono = true;
        }
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
        if self.idle_active && !ready.is_empty() {
            self.idle_filter(&mut ready);
        }
        self.phase(Phase::StartOfProcesses);
        // Not in the delta before the limit: its report names the processes of the last batch.
        if !self.fwd.is_empty() && !ready.is_empty() && self.delta + 1 < self.cfg.max_deltas {
            self.forward_copies(&mut ready);
        }
        if ready.is_empty() {
            // Nothing resumed (the falling edge of a clock, a transaction nobody waits for):
            // the batch would only commit the direct entries there are none of, and note that
            // no process ran.
            self.batch_mono = false;
            self.commit_direct();
            let mut old = std::mem::replace(&mut self.last_ran, ready);
            if self.ready.capacity() == 0 {
                old.clear();
                self.ready = old;
            }
        } else if let [p] = ready[..]
            && self.rng.is_none()
        {
            // `run_batch`'s own short path, without its entry (a stage of a chain).
            self.batch_mono = false;
            self.run_one(p, ready)?;
        } else {
            self.run_batch(ready, false, false)?;
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
            // Observers of the phases below may schedule something (a force, a wake-up): ask again
            // after them. Without any, `next` stands for `run_until` too.
            let bits = [
                Phase::LastKnownDelta,
                Phase::StartOfPostponed,
                Phase::EndOfTimeStep,
            ]
            .map(|p| 1u16 << p as u32)
            .into_iter()
            .fold(0, |m, b| m | b);
            let watched = self.phase_mask & bits != 0;
            self.phase(Phase::LastKnownDelta);
            if !watched || self.next_time() != Some(self.now) {
                let none = self.postponed.is_empty();
                if !none {
                    let postponed = std::mem::take(&mut self.postponed);
                    self.phase(Phase::StartOfPostponed);
                    self.run_batch(postponed, true, false)?;
                }
                if self.next_wanted && self.phase_mask & 1u16 << Phase::EndOfTimeStep as u32 != 0 {
                    self.obs_next = Some(self.next_time());
                }
                self.phase(Phase::EndOfTimeStep);
                self.obs_next = None;
                if !watched && none {
                    self.known_next = Some(next);
                }
            }
        }
        Ok(())
    }

    /// 14.7.5.3 b and c: sources, ports, forces, effective values and implicit signals.
    /// Records events in each signal's `changed`.
    fn update(&mut self) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        self.empty_delta = false;
        // The usual delta: direct transactions and drivers of one independent signal due, from
        // this time's list, in driver order; no force, deposit or implicit signal due (the
        // sources of implicit signals are looked at after it). Applied in place.
        if self.ops.is_empty() && self.queue.peek_time().is_none_or(|t| t > now) {
            let one = self.one_independent();
            if one.is_some() || self.queue.next_delta.is_empty() {
                self.stats.update_fast += 1;
                if let Some(s0) = one {
                    let sig = &mut self.signals[s0 as usize];
                    for e in &self.queue.next_delta {
                        if let Entry::Driver(d) = *e {
                            let info = &self.drivers[d as usize];
                            let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                            let w = &mut slot.st.waveforms[info.local as usize];
                            // A scalar register's transaction: applied here, not through the
                            // general entry.
                            match w.quick {
                                Some(q) if q.time <= now => {
                                    w.quick = None;
                                    self.stats.quick_applied += 1;
                                    sig.apply_quick(q, info.slot, now, cycle);
                                }
                                _ => sig.apply_driver(w, info.slot, now, cycle),
                            }
                        }
                    }
                    self.queue.next_delta.clear();
                    if finish_signal(sig, now, cycle)? {
                        self.changed_sigs.push(s0);
                    }
                }
                self.finish_direct()?;
                return self.update_implicit_quiet();
            }
        }
        if self.ops.is_empty()
            && self.queue.next_delta.is_empty()
            && let Some(r) = self.update_timed()
        {
            self.stats.update_timed += 1;
            return r;
        }
        self.stats.update_general += 1;
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
        // A signal with a force or deposit takes its direct transactions to the worklists.
        if !ops.is_empty() {
            let mut i = 0;
            while let Some(&s) = self.direct_sigs.get(i) {
                if !ops.contains_key(&s) {
                    i += 1;
                    continue;
                }
                self.direct_sigs.swap_remove(i);
                let sig = &mut self.signals[s as usize];
                sig.finish_pend(now, cycle, true);
                if sig.queued_up != cycle {
                    sig.queued_up = cycle;
                    up.push((sig.level, Reverse(s)));
                }
            }
        }
        // Matured drivers of independent signals without forces or deposits, as
        // (signal, driver) in queue order.
        let mut indep = std::mem::take(&mut self.indep_buf);
        // Everything due now, in (time, entry) order: the heap pops in order and holds nothing
        // before `now` (time only advances to its first entry), as the delta list.
        let mut entries = std::mem::take(&mut self.queue.next_delta);
        let from_delta = entries.len();
        while let Some(l) = self.queue.later.peek() {
            if l.time > now {
                break;
            }
            entries.push(l.entry());
            self.queue.later.pop();
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
        self.finish_direct()?;

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
            let sig_ops = ops.remove(&s).unwrap_or_default();
            if !sig_ops.is_empty() {
                // A release reads the drivers, a deposit makes the driving value differ.
                sig.sync_drivers();
                sig.exact |= sig_ops.iter().any(|op| op.mode == OpMode::Deposit);
            }
            let deposit = self.idle_active && sig_ops.iter().any(|op| op.mode == OpMode::Deposit);
            for op in sig_ops {
                apply_op(sig, &op, now, cycle)?;
            }
            if deposit {
                // The value a deposit leaves is not what the drivers' processes last assigned:
                // they assign again at their next edge.
                for d in &self.drivers {
                    if d.signal.0 == s {
                        self.armed[d.owner.0 as usize] = true;
                    }
                }
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
        if self.implicits.is_empty() || due.is_empty() && self.implicit_srcs_quiet() {
            due.clear();
            self.work = Some(work);
            return Ok(());
        }
        let r = self.update_implicit(due);
        due.clear();
        self.work = Some(work);
        r
    }

    /// The update of a cycle whose due entries are all in the future queue and are drivers of
    /// independent signals (the edge of a clock generator, the end of a delayed assignment) or
    /// timeouts: taken off the heap and applied in place, with no work lists. A few clocks that
    /// edge together are one such cycle. `None` (nothing done) for anything else: a timeout of
    /// an implicit signal, a port or group, a force, two drivers of one signal, too many.
    #[inline]
    fn update_timed(&mut self) -> Option<Result<(), ErrorKind>> {
        let now = self.now;
        let l = self.queue.later.peek()?;
        if l.time > now {
            return None;
        }
        if l.key >> 32 != 0 || self.queue.later.second_time().is_some_and(|t| t <= now) {
            return self.update_timed_many();
        }
        let d = l.key as u32;
        if !self.independent[self.drivers[d as usize].signal.0 as usize] {
            return None;
        }
        self.queue.later.pop();
        if let Err(e) = self.apply_timed(d) {
            return Some(Err(e));
        }
        Some(
            self.finish_direct()
                .and_then(|()| self.update_implicit_quiet()),
        )
    }

    /// Apply the matured quick or waveform transaction of driver `d` (of an independent signal)
    /// and finish its signal; a generator's driver re-arms itself (`rearm`).
    #[inline(always)]
    fn apply_timed(&mut self, d: u32) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        let info = &self.drivers[d as usize];
        let s0 = info.signal.0;
        let sig = &mut self.signals[s0 as usize];
        let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
        let w = &mut slot.st.waveforms[info.local as usize];
        let mut quick = None;
        match w.quick {
            Some(q) if q.time <= now => {
                w.quick = None;
                self.stats.quick_applied += 1;
                sig.apply_quick(q, info.slot, now, cycle);
                quick = Some(q);
            }
            _ => sig.apply_driver(w, info.slot, now, cycle),
        }
        // Nothing but the quick transaction: its event is all there is to finish.
        let fused = sig.end_apply();
        let changed = if sig.unfinished() || sig.has_pending() {
            finish_applied(sig, fused, now, cycle)?
        } else {
            fused
        };
        if changed {
            self.changed_sigs.push(s0);
        }
        if fused
            && let Some(q) = quick
            && let Some(&k) = self.toggle_ix.get(d as usize)
            && k != u32::MAX
        {
            self.rearm(k as usize, q.code);
        }
        Ok(())
    }

    /// `update_timed` for several entries due now (`TIMED_MAX` of them at most): drivers of
    /// distinct independent signals, applied in signal order as the general path does, and
    /// timeouts. Entries taken off the heap before one that does not fit go back on it.
    fn update_timed_many(&mut self) -> Option<Result<(), ErrorKind>> {
        const TIMED_MAX: usize = 4;
        let now = self.now;
        // (signal, driver) and (process, generation).
        let mut drv = [(0u32, 0u32); TIMED_MAX];
        let mut tmo = [(0u32, 0u64); TIMED_MAX];
        let (mut nd, mut nt) = (0, 0);
        while let Some(l) = self.queue.later.peek() {
            if l.time > now {
                break;
            }
            let fits = match l.key >> 32 {
                0 => {
                    let d = l.key as u32;
                    let s = self.drivers[d as usize].signal.0;
                    let ok = nd < TIMED_MAX
                        && self.independent[s as usize]
                        && drv[..nd].iter().all(|x| x.0 != s);
                    if ok {
                        drv[nd] = (s, d);
                        nd += 1;
                    }
                    ok
                }
                1 => {
                    let ok = nt < TIMED_MAX;
                    if ok {
                        tmo[nt] = (l.key as u32, l.generation);
                        nt += 1;
                    }
                    ok
                }
                _ => false,
            };
            if !fits {
                // Back on the heap, as they were: the general path takes all of them.
                for &(_, d) in &drv[..nd] {
                    self.queue.later.push(now, u64::from(d), 0);
                }
                for &(p, g) in &tmo[..nt] {
                    self.queue.later.push(now, 1 << 32 | u64::from(p), g);
                }
                return None;
            }
            self.queue.later.pop();
        }
        for &(p, g) in &tmo[..nt] {
            if self.procs[p as usize].generation == g {
                self.wake(p, true);
            }
        }
        drv[..nd].sort_unstable();
        // An error is taken at the first signal that has one, after all of them were updated.
        let mut failed = None;
        for &(_, d) in &drv[..nd] {
            if let Err(e) = self.apply_timed(d) {
                failed.get_or_insert(e);
            }
        }
        if let Some(e) = failed {
            return Some(Err(e));
        }
        Some(
            self.finish_direct()
                .and_then(|()| self.update_implicit_quiet()),
        )
    }

    /// The generator `k` made an event with its own transaction (the signal now has `code`):
    /// its process would resume, assign the complement after its delay and wait again.
    /// Do that here: the next transaction and its queue entry, and the process stays
    /// suspended (`skip_wake`). Only where the process's waiter stands for it and its driver
    /// is idle (the transaction just matured); else the process resumes as usual.
    fn rearm(&mut self, k: usize, code: u8) {
        let t = &self.toggles[k];
        let (d, p) = (t.driver, t.proc);
        let Ok(next) = self.now.checked_add(t.period) else {
            return;
        };
        if self.procs[p as usize].perm == 0 || self.skip_n == SKIP_MAX {
            return;
        }
        let info = &self.drivers[d as usize];
        let w = &mut slot_mut(&mut self.slots[info.owner.0 as usize])
            .st
            .waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return;
        }
        let elem = info.range.offset;
        w.quick = Some(crate::signal::Quick {
            time: next,
            elem,
            code: StdULogic::from_code_pub(code).not() as u8,
        });
        self.queue.later.push(next, u64::from(d), 0);
        // `wake` ignores a process that is ready already.
        self.procs[p as usize].ready = true;
        self.skip_wake[self.skip_n] = p;
        self.skip_n += 1;
    }

    /// The signal of `queue.next_delta` when every entry is a driver of that one independent
    /// signal, in driver order.
    fn one_independent(&self) -> Option<u32> {
        let Some(&Entry::Driver(d0)) = self.queue.next_delta.first() else {
            return None;
        };
        let s0 = self.drivers[d0 as usize].signal.0;
        if !self.independent[s0 as usize] {
            return None;
        }
        let mut prev = d0;
        for e in &self.queue.next_delta[1..] {
            match *e {
                Entry::Driver(d) if d >= prev && self.drivers[d as usize].signal.0 == s0 => {
                    prev = d
                }
                _ => return None,
            }
        }
        Some(s0)
    }

    /// The direct transactions of the processes committed in the batch loop, to their signals.
    fn commit_direct(&mut self) {
        if !self.direct.is_empty() || !self.directv.is_empty() || !self.directb.is_empty() {
            let sigs = &mut self.direct_sigs;
            let empty = &mut self.empty_delta;
            commit_out(
                &mut self.stats,
                &mut self.signals,
                sigs,
                empty,
                &mut self.direct,
                &mut self.directv,
                &mut self.directb,
            );
        }
    }

    /// Finish the signals with direct transactions (`commit_out`; one that a driver entry
    /// finished already has nothing left).
    fn finish_direct(&mut self) -> Result<(), ErrorKind> {
        let (now, cycle) = (self.now, self.cycle);
        // One element of one signal (a stage of a chain): no loop over the general cases.
        if let [s] = self.direct_sigs[..]
            && let Some(changed) = self.signals[s as usize].finish_pend_one(now, cycle)
        {
            self.direct_sigs.clear();
            if changed && !(!self.watch_changes && self.wake_sole(s)) {
                self.changed_sigs.push(s);
            }
            return Ok(());
        }
        for i in 0..self.direct_sigs.len() {
            let s = self.direct_sigs[i];
            let sig = &mut self.signals[s as usize];
            let mut changed = sig.finish_pend(now, cycle, false) == Some(true);
            // A force layer made dirty ranges of them.
            if sig.unfinished() {
                changed |= finish_signal(sig, now, cycle)?;
            }
            if changed {
                // After runs that driver entries of the signal made.
                if sig.changed.len() > 1 {
                    sig.canonical_changed();
                }
                if !self.watch_changes && self.wake_sole(s) {
                    continue;
                }
                self.changed_sigs.push(s);
            }
        }
        self.direct_sigs.clear();
        Ok(())
    }

    /// The resumption of `cycle` for signal `s` done at once when one element of it changed and
    /// one resident process without an edge trigger waits for that element (a stage of a
    /// chain), as the reference simulator wakes the waiters of a value when it changes. False
    /// (nothing done) otherwise: `cycle` looks at the signal. Only with no observer of value
    /// changes, which must see the change before any process is resumed.
    #[inline(always)]
    fn wake_sole(&mut self, s: u32) -> bool {
        // A GUARD that other guards read stays in `changed_sigs` (`run_guards`).
        if !self.guard_readers.is_empty() && self.guard_readers.contains_key(&s) {
            return false;
        }
        if self.idle_active {
            self.arm_readers(s);
        }
        let sig = &self.signals[s as usize];
        let [r] = sig.changed[..] else {
            return false;
        };
        let Some(w) = (r.len == 1)
            .then(|| self.waiters[s as usize].sole_resident(r.offset))
            .flatten()
        else {
            return false;
        };
        // An edge trigger reads signals this update may not have finished yet.
        if self.cfg.edge_triggers && w.tid != 0 || !alive(&self.procs, w) {
            return false;
        }
        let p = w.proc;
        let sig = &mut self.signals[s as usize];
        self.stats.events += 1 + std::mem::take(&mut sig.nruns);
        sig.changed.clear();
        self.wake(p, false);
        true
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
        // Several signals and no pool to hand them to (or too few for it): one after the other,
        // with no job and result lists. An error is taken at the first signal that has one, after
        // all of them were updated, as below.
        // ponytail: the cut-off is the pool's below.
        let helpers = if self.pool.is_none() || due.len() < 256 {
            0
        } else {
            self.helpers()
        };
        if helpers == 0 {
            let mut failed = None;
            for run in due.chunk_by(|a, b| a.0 == b.0) {
                let s = run[0].0 as usize;
                let sig = self
                    .signals
                    .get_mut(s)
                    .ok_or_else(|| ErrorKind::Internal(format!("driver of missing signal {s}")))?;
                for &(_, d) in run {
                    let info = &self.drivers[d as usize];
                    let slot = slot_mut(&mut self.slots[info.owner.0 as usize]);
                    sig.apply_driver(
                        &mut slot.st.waveforms[info.local as usize],
                        info.slot,
                        now,
                        cycle,
                    );
                }
                match finish_signal(sig, now, cycle) {
                    Ok(true) => self.changed_sigs.push(s as u32),
                    Ok(false) => {}
                    Err(e) => _ = failed.get_or_insert(e),
                }
            }
            return failed.map_or(Ok(()), Err);
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
                // With a token pool, at most one piece per thread it gave.
                let min = match self.tokens {
                    Some(_) => jobs.len().div_ceil(helpers + 1),
                    None => 1,
                };
                pool.install(|| jobs.par_iter_mut().with_min_len(min).map(job).collect())
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

    /// Whether no source of an implicit signal had activity in this cycle.
    #[inline]
    fn implicit_srcs_quiet(&self) -> bool {
        let cycle = self.cycle;
        self.implicit_srcs
            .iter()
            .all(|&(s, r)| !self.signals[s as usize].active_in(r, cycle))
    }

    /// The implicit signals after an update of the fast paths (which run with implicit signals
    /// in the design): none is due (the paths take no queue entry of one), so only a source that
    /// had activity can change one.
    /// The test is inlined into the fast paths, a design with implicit signals pays the call.
    #[inline(always)]
    fn update_implicit_quiet(&mut self) -> Result<(), ErrorKind> {
        if self.implicits.is_empty() {
            return Ok(());
        }
        self.update_implicit_active()
    }

    #[inline(never)]
    fn update_implicit_active(&mut self) -> Result<(), ErrorKind> {
        if self.implicit_srcs_quiet() {
            return Ok(());
        }
        self.update_implicit(&BTreeSet::new())
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

    /// Whether the cached wake list of signal `s` can run as one batch (`Waiters::cache_batch`,
    /// worked out once per list): its processes with one suspension point that are not
    /// postponed, in ascending order, run in the batch as listed; the others wake one by one.
    fn batch_of(&mut self, s: usize, list: &[(u64, u32, u32)]) -> bool {
        if let Some(b) = self.waiters[s].cache_batch() {
            return b.is_some();
        }
        let procs = &self.procs;
        let plain = |p: u32| {
            let m = &procs[p as usize];
            !m.flags.postponed && !m.flags.several_waits && m.perm != 0
        };
        let mut b = crate::signal::Batch {
            plain: Vec::new(),
            by_tid: SmallVec::new(),
            others: Vec::new(),
        };
        for &(_, p, tid) in list {
            if plain(p) {
                b.plain.push(p);
                match b.by_tid.iter_mut().find(|(t, _)| *t == tid) {
                    Some((_, l)) => l.push(p),
                    None => b.by_tid.push((tid, vec![p])),
                }
            } else {
                b.others.push((p, tid));
            }
        }
        // They run in process order whatever the order they registered in (`order` sorts a
        // batch of such processes), and once each.
        b.plain.sort_unstable();
        b.plain.dedup();
        for (_, l) in &mut b.by_tid {
            l.sort_unstable();
            l.dedup();
        }
        let ok = !b.plain.is_empty();
        self.waiters[s].set_cache_batch(ok.then_some(b));
        ok
    }

    #[inline(always)]
    fn wake(&mut self, p: u32, by_timeout: bool) {
        wake_proc(
            &mut self.procs,
            &mut self.postponed,
            &mut self.ready_several,
            &mut self.ready_mono,
            &mut self.ready,
            &mut self.stats.wakes,
            p,
            by_timeout,
        );
    }

    /// The order in which a batch runs (serial processes) and commits (#252, user
    /// 2026-10-19): at initialisation elaboration order; afterwards the processes with one
    /// suspension point in elaboration order, then the others in reverse elaboration order,
    /// those resumed by a signal before those resumed by a timeout.
    fn order(&mut self, pids: &mut [u32], init: bool) {
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
        // (reversed for several-wait processes except the settled ones) in the low 32. A key
        // names its process, so equal keys are equal processes and the sort need not be stable.
        const SETTLED: u64 = 1;
        let procs = &self.procs;
        let key = |p: u32| {
            let m = &procs[p as usize];
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
        let mut keyed = std::mem::take(&mut self.order_keys);
        keyed.clear();
        keyed.extend(pids.iter().map(|&p| key(p)));
        if !keyed.is_sorted() {
            keyed.sort_unstable();
            for (p, &k) in pids.iter_mut().zip(&keyed) {
                *p = if k >> 51 != 0 && (k >> 48) & 3 != SETTLED {
                    !(k as u32)
                } else {
                    k as u32
                };
            }
        }
        self.order_keys = keyed;
    }

    /// Runs of the concurrent copies in `ready` (#519): a copy whose waits stay (resident) and
    /// whose drivers have nothing pending does nothing but commit its sources' values to its direct
    /// drivers, so the kernel does that here, as its commit would, and drops it from the batch.
    fn forward_copies(&mut self, ready: &mut Vec<u32>) {
        let mut keep = 0;
        for i in 0..ready.len() {
            let p = ready[i];
            if !self.forward_copy(p) {
                ready[keep] = p;
                keep += 1;
            }
        }
        ready.truncate(keep);
    }

    #[inline]
    fn forward_copy(&mut self, p: u32) -> bool {
        let Some(Some(parts)) = self.fwd.get(p as usize) else {
            return false;
        };
        if self.procs[p as usize].perm == 0 {
            return false;
        }
        // Its zero-delay drives never leave anything in its waveforms past the next update.
        debug_assert!({
            let st = &slot_mut(&mut self.slots[p as usize]).st;
            parts.iter().all(|f| {
                st.waveforms
                    .get(f.local as usize)
                    .is_some_and(|w| w.idle() && w.phantom.is_empty())
            })
        });
        for f in parts.iter() {
            if f.es == 0 {
                let Buf::Planes { b, .. } = &self.signals[f.src as usize].cur else {
                    unreachable!("copy part checked at build: logic source");
                };
                let block = |k: u32| {
                    let n = (f.len - k).min(64);
                    let planes =
                        yavs_logic::wide::get_block(b, (f.src_off + k) as usize, n as usize)
                            .map(|w| w & nmask(n as usize));
                    OutV {
                        sig: f.dst,
                        slot: f.slot,
                        elem: f.dst_off + k,
                        len: n,
                        planes,
                    }
                };
                let first = if f.len <= 64 {
                    let o = block(0);
                    self.signals[f.dst as usize].commit_planes(o.slot, o.elem, o.len, o.planes)
                } else {
                    // A bus: its blocks as one run, the signal's storage looked up once.
                    let mut run = std::mem::take(&mut self.fwd_run);
                    run.clear();
                    run.extend((0..f.len).step_by(64).map(block));
                    let first = self.signals[f.dst as usize].commit_planes_run(&run);
                    self.fwd_run = run;
                    first
                };
                if first {
                    self.direct_sigs.push(f.dst);
                }
            } else {
                let es = f.es as usize;
                let Buf::Bytes { data, .. } = &self.signals[f.src as usize].cur else {
                    unreachable!("copy part checked at build: byte source");
                };
                let at = f.src_off as usize * es;
                let mut val = [0u8; 8];
                val[..es].copy_from_slice(&data[at..at + es]);
                match self.signals[f.dst as usize].commit_bytes(f.slot, f.dst_off, es, &val) {
                    Some(true) => self.direct_sigs.push(f.dst),
                    Some(false) => {}
                    None => self.empty_delta = true,
                }
            }
        }
        let m = &mut self.procs[p as usize];
        m.ready = false;
        m.resumed = true;
        self.stats.process_runs += 1;
        self.stats.copy_forwards += 1;
        true
    }

    /// Run `pids` (in parallel where allowed), then commit their effects and waits in
    /// `order`.
    fn run_batch(
        &mut self,
        mut pids: Vec<u32>,
        postponed: bool,
        init: bool,
    ) -> Result<(), SimError> {
        if let [p] = pids[..]
            && !init
            && !postponed
            && self.rng.is_none()
        {
            self.batch_mono = false;
            return self.run_one(p, pids);
        }
        if let Some(state) = &mut self.rng {
            shuffle(&mut pids, state);
        } else if !std::mem::take(&mut self.batch_mono) {
            self.order(&mut pids, init);
        }
        // The processes the commit loop has to commit, in batch order: those not committed right
        // after they ran.
        let mut late = std::mem::take(&mut self.late);
        // Initialisation and postponed processes always run inline (their commit differs).
        // The estimate of a pooled batch, to learn from its time (`learn_pooled`).
        let mut est = if self.pool.is_some() && !init && !postponed {
            self.pool_batch(&pids)
        } else {
            None
        };
        // With a token pool and no token, the batch runs inline.
        let helpers = if est.is_some() { self.helpers() } else { 0 };
        if est.is_some() && helpers == 0 {
            est = None;
        } else if est.is_none() && self.held != 0 {
            self.tokens_idle();
        }
        self.stats.pooled_batches += u64::from(est.is_some());
        let pooled = est.is_some();
        // A batch on fewer threads than the pool has teaches nothing about the threshold.
        let full = self
            .pool
            .as_ref()
            .is_some_and(|p| p.current_num_threads() == helpers);
        let pooled_at = est
            .filter(|e| e.learn && full)
            .map(|_| std::time::Instant::now());
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
                interrupt: &self.interrupt.0,
            };
            // Reports go straight to the output when the commit order is the order they are
            // made in: no shuffled run, and (below) nothing earlier in the batch to write.
            let live_ok = self.rng.is_none() && self.live_sink.is_some();
            // An error ends the commit at its process: later ones must not write.
            let stops = |failed: bool, slot: &Slot| {
                failed
                    || postponed
                        && (slot.st.deadline == Some(sh.now)
                            || !slot.st.phantoms.is_empty()
                            || !slot.st.out.is_empty()
                            || !slot.st.outv.is_empty()
                            || !slot.st.outb.is_empty()
                            || slot.st.scheduled.iter().any(|&(t, _)| t == sh.now))
            };
            let run = |p: u32, slot: &mut Slot, live: bool| {
                let mut ctx = Ctx {
                    sh: &sh,
                    pid: ProcId(p),
                    st: &mut slot.st,
                    live,
                };
                // A panic in process code is a broken invariant: report it as an internal
                // error of that process, not a bare panic.
                slot.result = catch_unwind(AssertUnwindSafe(|| slot.process.run(&mut ctx)))
                    .unwrap_or_else(|panic| Err(process_panic(&sh, p, &*panic)));
            };

            match &self.pool {
                // The batch cut into pieces of consecutive process numbers, each with its own
                // sub-slices of the slots, metas and idle flags (no lock per process); the pool's
                // threads and this one claim pieces in turn. A process that waited again and only
                // scheduled drivers is committed in the worker as in the inline loop below, into
                // the piece's buffers; the others are committed after the batch in batch order.
                // Serial processes run after the pieces, here, in batch order
                // (`docs/design/parallel-kernel.md` 4.2).
                Some(pool) if pooled => {
                    let mut sorted = pids.clone();
                    if !sorted.is_sorted() {
                        sorted.sort_unstable();
                    }
                    let k = helpers + 1;
                    let per = sorted.len().div_ceil(k * PIECES_PER_THREAD);
                    let (mut slots, mut procs, mut armed) = (
                        &mut self.slots[..],
                        &mut self.procs[..],
                        &mut self.armed[..],
                    );
                    let mut base = 0;
                    let mut parts = Vec::with_capacity(sorted.len().div_ceil(per));
                    let mut bufs = std::mem::take(&mut self.early);
                    for c in sorted.chunks(per) {
                        let (lo, hi) = (c[0] as usize, c[c.len() - 1] as usize + 1);
                        let (s, rest) =
                            std::mem::take(&mut slots)[lo - base..].split_at_mut(hi - lo);
                        slots = rest;
                        let (m, rest) =
                            std::mem::take(&mut procs)[lo - base..].split_at_mut(hi - lo);
                        procs = rest;
                        let (a, rest) =
                            std::mem::take(&mut armed)[lo - base..].split_at_mut(hi - lo);
                        armed = rest;
                        base = hi;
                        parts.push(Mutex::new(Piece {
                            pids: c,
                            lo,
                            slots: s,
                            procs: m,
                            armed: a,
                            out: bufs.pop().unwrap_or_default(),
                        }));
                    }
                    let (next, names) = (AtomicUsize::new(0), &self.names);
                    let work = || {
                        while let Some(part) = parts.get(next.fetch_add(1, Ordering::Relaxed)) {
                            let mut part = part.lock().unwrap_or_else(PoisonError::into_inner);
                            let Piece {
                                pids,
                                lo,
                                slots,
                                procs,
                                armed,
                                out,
                            } = &mut *part;
                            for &p in pids.iter() {
                                let i = p as usize - *lo;
                                if procs[i].flags.serial {
                                    out.serial += 1;
                                    out.late.push(p);
                                    continue;
                                }
                                let slot = slot_mut(&mut slots[i]);
                                resumption(procs, i as u32, &mut slot.st);
                                run(p, slot, false);
                                let (st, meta) = (&mut slot.st, &mut procs[i]);
                                // As the early commit of the inline loop below.
                                if st.again
                                    && st.act & ACT_SLOW == 0
                                    && meta.perm != 0
                                    && st.wait_key == meta.wkey
                                    && slot.result.is_ok()
                                {
                                    st.check_flags();
                                    st.again = false;
                                    st.matched = 0;
                                    st.wait_key = crate::process::NO_EDGE;
                                    meta.ready = false;
                                    meta.resumed = true;
                                    if meta.idle_ok
                                        && note_idle(
                                            meta,
                                            &mut armed[i],
                                            sh.signals,
                                            &names[p as usize],
                                            st,
                                        )
                                    {
                                        out.idle_done += 1;
                                    }
                                    if st.act & ACT_ASSIGNED != 0 {
                                        out.queue
                                            .extend(st.scheduled.drain(..).map(|(t, d)| (t, d.0)));
                                        out.direct.append(&mut st.out);
                                        out.directv.append(&mut st.outv);
                                        out.directb.append(&mut st.outb);
                                        if !st.phantoms.is_empty() {
                                            out.empty_delta |= st.take_phantoms();
                                        }
                                    }
                                } else {
                                    out.failed |= slot.result.is_err();
                                    out.late.push(p);
                                }
                            }
                        }
                    };
                    pool.in_place_scope(|sc| {
                        for _ in 1..k {
                            sc.spawn(|_| work());
                        }
                        work();
                    });
                    // The pieces in process order, so their late lists join in process order.
                    let (mut lates, mut serial, mut failed) = (Vec::new(), 0, false);
                    for part in parts {
                        let mut out = part
                            .into_inner()
                            .unwrap_or_else(PoisonError::into_inner)
                            .out;
                        for (t, d) in out.queue.drain(..) {
                            self.queue.push(self.now, t, Entry::Driver(d));
                        }
                        self.direct.append(&mut out.direct);
                        self.directv.append(&mut out.directv);
                        self.directb.append(&mut out.directb);
                        self.empty_delta |= std::mem::take(&mut out.empty_delta);
                        self.idle_live -= std::mem::take(&mut out.idle_done);
                        serial += std::mem::take(&mut out.serial);
                        failed |= std::mem::take(&mut out.failed);
                        lates.append(&mut out.late);
                        bufs.push(out);
                    }
                    self.early = bufs;
                    self.idle_active = self.idle_live != 0;
                    // The serial processes in batch order, as the inline loop runs them: none
                    // after a process that stopped the batch (it is not committed either), and
                    // live while no earlier one wrote.
                    let mut unrun = 0;
                    if serial != 0 || failed {
                        let mut live = live_ok;
                        for (n, &p) in pids.iter().enumerate() {
                            let ser = self.serial_of[p as usize];
                            let s = slot_mut(&mut self.slots[p as usize]);
                            if ser && serial != 0 {
                                resumption(&self.procs, p, &mut s.st);
                                run(p, s, live);
                            }
                            if stops(s.result.is_err(), s) {
                                unrun = n + 1;
                                break;
                            }
                            live &= ser
                                || !(s.st.effects.iter())
                                    .any(|e| matches!(e, Effect::Report(..) | Effect::Output(..)));
                        }
                    }
                    // After a stop, as inline: nothing later in the batch is committed.
                    let ran = if unrun == 0 {
                        &pids[..]
                    } else {
                        &pids[..unrun]
                    };
                    if lates.len() == pids.len() && unrun == 0 {
                        late.extend_from_slice(&pids);
                    } else if !lates.is_empty() {
                        late.extend(ran.iter().filter(|p| lates.binary_search(p).is_ok()));
                    }
                }
                // No other thread touches the slots: no lock. One unwind guard for the batch
                // (`i` is the process that panicked): a panic ends the batch at that process,
                // as a failed run does.
                _ => {
                    let now = self.now;
                    let (mut i, mut at) = (0, 0);
                    // A big batch misses the cache on every process: ask for the hot lines of
                    // a process a few steps ahead (#766).
                    let ahead = if pids.len() >= PREFETCH_BATCH {
                        PREFETCH_AHEAD
                    } else {
                        usize::MAX
                    };
                    // With a pool: one activation in `COST_EVERY` of each process is timed for
                    // `pool_batch`, at a position that moves from batch to batch.
                    // The offset is scrambled so batches that come every 2^k batches still see
                    // every position.
                    let mut sample = if self.pool.is_some() {
                        (self.batches.wrapping_mul(0x9e37_79b9) >> 28) as usize % COST_EVERY
                    } else {
                        usize::MAX
                    };
                    // `$timed`: the loop that times activations, a second copy so the one of a
                    // run without a pool has no test for it.
                    macro_rules! batch_loop {
                        ($timed:literal) => {
                            catch_unwind(AssertUnwindSafe(|| {
                                while let Some(&p) = pids.get(i) {
                                    at = i;
                                    if let Some(&n) = pids.get(i.saturating_add(ahead)) {
                                        let next = &mut self.slots[n as usize];
                                        // The slot (the process's kernel state), then the process.
                                        yavs_prefetch::lines(&*next, SLOT_LINES);
                                        yavs_prefetch::lines(&*slot_mut(next).process, PREFETCH_LINES);
                                    }
                                    let s = slot_mut(&mut self.slots[p as usize]);
                                    if !init {
                                        resumption(&self.procs, p, &mut s.st);
                                    }
                                    let mut ctx = Ctx {
                                        sh: &sh,
                                        pid: ProcId(p),
                                        st: &mut s.st,
                                        live: live_ok,
                                    };
                                    // `result` stays `Ok` unless a run failed (a store would drop the old one).
                                    let r = if !($timed && i == sample) {
                                        s.process.run(&mut ctx)
                                    } else {
                                        sample += COST_EVERY;
                                        let t = std::time::Instant::now();
                                        let r = s.process.run(&mut ctx);
                                        let ns = t.elapsed().as_nanos().min(u128::from(u32::MAX)) as u32;
                                        let c = &mut self.cost[p as usize];
                                        *c = *c / 2 + ns / 2;
                                        r
                                    };
                                    let failed = r.is_err();
                                    if failed {
                                        s.result = r;
                                    }
                                    if stops(failed, s) {
                                        late.push(p);
                                        break;
                                    }
                                    i += 1;
                                    // The usual clocked process, committed while its slot is hot: it waited
                                    // again (`Ctx::wait_on_again`) and only scheduled drivers. The commit
                                    // loop skips it (`ProcMeta::ready` is false). The order of the queue
                                    // pushes across processes does not matter: the entries are sorted.
                                    let st = &mut s.st;
                                    let meta = &mut self.procs[p as usize];
                                    if !init
                                        && !postponed
                                        && st.again
                                        && st.act & ACT_SLOW == 0
                                        && meta.perm != 0
                                        && st.wait_key == meta.wkey
                                        && !failed
                                    {
                                        st.check_flags();
                                        st.again = false;
                                        st.matched = 0;
                                        st.wait_key = crate::process::NO_EDGE;
                                        meta.ready = false;
                                        meta.resumed = true;
                                        self.serial_ran |= meta.flags.serial;
                                        if meta.idle_ok
                                            && note_idle(
                                                meta,
                                                &mut self.armed[p as usize],
                                                sh.signals,
                                                &self.names[p as usize],
                                                st,
                                            )
                                        {
                                            self.idle_live -= 1;
                                            self.idle_active = self.idle_live != 0;
                                        }
                                        // A quiet activation (nothing assigned) has nothing to commit.
                                        if st.act & ACT_ASSIGNED != 0 {
                                            for &(t, d) in st.scheduled.iter() {
                                                self.queue.push(now, t, Entry::Driver(d.0));
                                            }
                                            st.scheduled.clear();
                                            // Applied after the batch: the signals are shared until then.
                                            if let [o] = st.out[..] {
                                                self.direct.push(o);
                                                st.out.clear();
                                            } else if !st.out.is_empty() {
                                                self.direct.extend_from_slice(&st.out);
                                                st.out.clear();
                                            }
                                            if let [o] = st.outv[..] {
                                                // One entry (the usual register): a store, not a `memcpy`.
                                                self.directv.push(o);
                                                st.outv.clear();
                                            } else if !st.outv.is_empty() {
                                                self.directv.extend_from_slice(&st.outv);
                                                st.outv.clear();
                                            }
                                            if !st.outb.is_empty() {
                                                self.directb.extend_from_slice(&st.outb);
                                                st.outb.clear();
                                            }
                                            if !st.phantoms.is_empty() {
                                                self.empty_delta |= st.take_phantoms();
                                            }
                                        }
                                    } else {
                                        late.push(p);
                                    }
                                }
                            }))
                        };
                    }
                    let caught = if self.pool.is_some() {
                        batch_loop!(true)
                    } else {
                        batch_loop!(false)
                    };
                    if let Err(panic) = caught {
                        // The process that was running or being committed (`i` is past it by then).
                        let p = pids[at];
                        late.push(p);
                        slot_mut(&mut self.slots[p as usize]).result =
                            Err(process_panic(&sh, p, &*panic));
                    }
                }
            }
        }
        if let (Some(t), Some(e)) = (pooled_at, est) {
            self.learn_pooled(e, t.elapsed().as_nanos() as u64);
        }
        self.commit_rest(late, pids, postponed, init)
    }

    /// The time of a pooled batch over its estimate inline, averaged (`pool_ratio`, 1024 is
    /// even): when the average reaches even, the threshold of `pool_batch` rises to twice the
    /// batch's splittable work; a batch at least a fifth faster lowers it to its splittable
    /// work, not below `POOL_WORK_NS`. Processes with much state lose more to the caches of
    /// other cores than the estimate shows (a processor's pipeline, #497); the average keeps a
    /// late worker or a preempted core from raising it.
    fn learn_pooled(&mut self, e: Estimate, ns: u64) {
        let r = (ns.saturating_mul(1024) / e.all.max(1)).min(8 * 1024);
        self.pool_ratio = self.pool_ratio - self.pool_ratio / 16 + r / 16;
        if self.pool_ratio >= 1024 {
            self.pool_ns = self.pool_ns.max(e.work.saturating_mul(2));
        } else if r < 1024 * 4 / 5 {
            self.pool_ns = self.pool_ns.min(e.work).max(POOL_WORK_NS);
        }
    }

    /// Whether a batch goes to the pool, with its estimate: the estimated work of its parallel
    /// processes, less the largest one (a batch whose time is one process cannot be split),
    /// passes `pool_ns`. Costs are learnt from inline runs only (a pooled run measures cold
    /// caches and would confirm itself), so one in `PROBE_EVERY` of the batches that would pool
    /// runs inline; and one in `PROBE_EVERY` of those between `POOL_WORK_NS` and a raised
    /// `pool_ns` is pooled, so a threshold raised early can come down (`learn_pooled`). Where a
    /// batch runs changes no output: both paths commit in batch order (#497,
    /// `docs/design/parallel-kernel.md` 4.1).
    fn pool_batch(&mut self, pids: &[u32]) -> Option<Estimate> {
        if self.cfg.pool_all {
            return (pids.len() >= 2).then_some(Estimate::default());
        }
        self.batches = self.batches.wrapping_add(1);
        let (mut all, mut sum, mut max, mut known) = (0, 0, 0, true);
        for &p in pids {
            let c = u64::from(self.cost[p as usize]);
            all += c;
            known &= c != 0;
            if !self.serial_of[p as usize] {
                sum += c;
                max = max.max(c);
            }
        }
        let work = sum - max;
        let pool = if work > self.pool_ns {
            self.probe = self.probe.wrapping_add(1);
            !self.probe.is_multiple_of(PROBE_EVERY)
        } else if work > POOL_WORK_NS {
            self.raise_probe = self.raise_probe.wrapping_add(1);
            self.raise_probe.is_multiple_of(PROBE_EVERY)
        } else {
            false
        };
        // A batch with a process not yet timed teaches nothing.
        pool.then_some(Estimate {
            all,
            work,
            learn: known,
        })
    }

    /// `run_batch` of one process after initialisation (a stage of a chain of processes): no
    /// order, no batch loop, and the usual clocked commit straight from its own lists.
    fn run_one(&mut self, p: u32, pids: Vec<u32>) -> Result<(), SimError> {
        self.stats.single_runs += 1;
        if self.held != 0 {
            self.tokens_idle();
        }
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
            interrupt: &self.interrupt.0,
        };
        let s = slot_mut(&mut self.slots[p as usize]);
        resumption(&self.procs, p, &mut s.st);
        let mut ctx = Ctx {
            sh: &sh,
            pid: ProcId(p),
            st: &mut s.st,
            live: sh.live.is_some(),
        };
        // A panic in process code is a broken invariant: an internal error of the process.
        let r = catch_unwind(AssertUnwindSafe(|| s.process.run(&mut ctx)))
            .unwrap_or_else(|panic| Err(process_panic(&sh, p, &*panic)));
        // `result` stays `Ok` unless a run failed (a store would drop the old one).
        let failed = r.is_err();
        if failed {
            s.result = r;
        }
        let st = &mut s.st;
        let meta = &mut self.procs[p as usize];
        // The usual clocked process (see `run_batch`): committed here.
        if !(st.again
            && st.act & ACT_SLOW == 0
            && meta.perm != 0
            && st.wait_key == meta.wkey
            && !failed)
        {
            let mut late = std::mem::take(&mut self.late);
            late.push(p);
            return self.commit_rest(late, pids, false, false);
        }
        st.check_flags();
        st.again = false;
        st.matched = 0;
        st.wait_key = crate::process::NO_EDGE;
        meta.ready = false;
        meta.resumed = true;
        self.serial_ran |= meta.flags.serial;
        if meta.idle_ok
            && note_idle(
                meta,
                &mut self.armed[p as usize],
                sh.signals,
                &self.names[p as usize],
                st,
            )
        {
            self.idle_live -= 1;
            self.idle_active = self.idle_live != 0;
        }
        for &(t, d) in st.scheduled.iter() {
            self.queue.push(self.now, t, Entry::Driver(d.0));
        }
        st.scheduled.clear();
        if !st.phantoms.is_empty() {
            self.empty_delta |= st.take_phantoms();
        }
        if let [o] = st.out[..]
            && st.outv.is_empty()
            && st.outb.is_empty()
        {
            // One element (a stage of a chain): `commit_out` without its loops.
            st.out.clear();
            self.stats.direct_codes += 1;
            match self.signals[o.sig as usize].commit_code(o.slot, o.elem, o.code as u8) {
                Some(true) => self.direct_sigs.push(o.sig),
                Some(false) => {}
                None => self.empty_delta = true,
            }
        } else {
            commit_out(
                &mut self.stats,
                &mut self.signals,
                &mut self.direct_sigs,
                &mut self.empty_delta,
                &mut st.out,
                &mut st.outv,
                &mut st.outb,
            );
        }
        self.stats.process_runs += 1;
        let mut old = std::mem::replace(&mut self.last_ran, pids);
        if self.ready.capacity() == 0 {
            old.clear();
            self.ready = old;
        }
        Ok(())
    }

    /// The end of `run_batch` after the run phase: the direct entries of the processes
    /// committed in the batch loop, then the commit of the others (`late`) in batch order.
    fn commit_rest(
        &mut self,
        mut late: Vec<u32>,
        pids: Vec<u32>,
        postponed: bool,
        init: bool,
    ) -> Result<(), SimError> {
        self.commit_direct();
        self.stats.process_runs += pids.len() as u64;
        if late.is_empty() {
            // Every process was committed after it ran: nothing left for the commit loop.
            self.late = late;
            let mut old = std::mem::replace(&mut self.last_ran, pids);
            if self.ready.capacity() == 0 {
                old.clear();
                self.ready = old;
            }
            return Ok(());
        }
        // The batch is in commit order already unless it was shuffled to run.
        if self.rng.is_some() {
            self.order(&mut late, init);
        }
        let mut first_err = None;
        for &p in &late {
            let meta = &mut self.procs[p as usize];
            meta.ready = false;
            meta.resumed |= !init;
            self.serial_ran |= meta.flags.serial;
            let (generation, mut trigger, mut tid) = (meta.generation, meta.trigger, meta.tid);
            let slot = slot_mut(&mut self.slots[p as usize]);
            let st = &mut slot.st;
            // A wait that resumes the process only on an edge (`Ctx::wait_edge`): its waiters
            // carry that edge's trigger, and a wait with another one registers them again.
            let wkey = std::mem::replace(&mut st.wait_key, crate::process::NO_EDGE);
            let wkey = if tid != 0 || !self.cfg.edge_triggers {
                crate::process::NO_EDGE
            } else {
                wkey
            };
            if wkey != crate::process::NO_EDGE {
                let t = dyn_trigger(wkey);
                tid = *self.trig_ids.entry(t).or_insert_with(|| {
                    self.triggers.push(t);
                    self.triggers.len() as u32
                });
                trigger = Some(t);
            }
            // A plain activation (below) of a clocked process the idle skip tracks: record what
            // it did, as the batch loop does, before the commit takes its lists.
            if meta.idle_ok
                && !init
                && !postponed
                && st.again
                && meta.perm != 0
                && st.sens.is_empty()
                && st.deadline.is_none()
                && st.effects.is_empty()
                && slot.result.is_ok()
            {
                if note_idle(
                    meta,
                    &mut self.armed[p as usize],
                    &self.signals,
                    &self.names[p as usize],
                    st,
                ) {
                    self.idle_live -= 1;
                    self.idle_active = self.idle_live != 0;
                }
            } else {
                meta.idle_state = 0;
                meta.shadow = false;
            }
            // The usual clocked process: it waited again on the range its resident waiter
            // already covers (`Ctx::wait_on_again`) and did nothing but schedule drivers.
            if st.again
                && !postponed
                && meta.perm != 0
                && wkey == meta.wkey
                && st.sens.is_empty()
                && st.deadline.is_none()
                && st.effects.is_empty()
                && st.phantoms.is_empty()
                && slot.result.is_ok()
            {
                st.again = false;
                st.matched = 0;
                for &(t, d) in st.scheduled.iter() {
                    self.queue.push(self.now, t, Entry::Driver(d.0));
                }
                st.scheduled.clear();
                commit_out(
                    &mut self.stats,
                    &mut self.signals,
                    &mut self.direct_sigs,
                    &mut self.empty_delta,
                    &mut st.out,
                    &mut st.outv,
                    &mut st.outb,
                );
                continue;
            }
            let effects = (!st.effects.is_empty()).then(|| std::mem::take(&mut st.effects));
            let bad = postponed
                && (st.deadline == Some(self.now)
                    || !st.phantoms.is_empty()
                    || !st.out.is_empty()
                    || !st.outv.is_empty()
                    || !st.outb.is_empty()
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
            commit_out(
                &mut self.stats,
                &mut self.signals,
                &mut self.direct_sigs,
                &mut self.empty_delta,
                &mut st.out,
                &mut st.outv,
                &mut st.outb,
            );
            let timed = st.deadline.is_some();
            if let Some(t) = st.deadline.take() {
                self.queue.push(self.now, t, Entry::Timeout(p, generation));
            }
            // The same wait as the last run, without a timeout: the waiters of the last run
            // stay (resident, `ProcMeta::perm`), or become so now.
            // A wait that ended after naming a part of the last one again: that part first.
            if st.matched != 0 && !st.again {
                let k = st.matched as usize;
                let mut all = st.sens_prev[..k].to_vec();
                all.append(&mut st.sens);
                st.sens = all;
            }
            st.matched = 0;
            let mut again = std::mem::take(&mut st.again);
            if again && (timed || !st.sens.is_empty()) {
                // More was added to the wait (`Ctx::wait_on_again`): the last wait's ranges
                // come first, as they were first named.
                let mut all = st.sens_prev.clone();
                all.append(&mut st.sens);
                st.sens = all;
                again = false;
            }
            // The resident waiters carry the trigger of the last wait: another one names the
            // wait in full and registers them again.
            let moved = wkey != meta.wkey;
            if moved && again {
                st.sens = st.sens_prev.clone();
                again = false;
            }
            meta.wkey = wkey;
            let same = !timed
                && !moved
                && (again || !st.sens.is_empty() && st.sens[..] == st.sens_prev[..]);
            let mut perm = 0;
            // The ranges to register: `sens`, or `sens_prev` when they are the same or the new
            // wait became the previous one (the buffers swap, nothing is copied).
            let mut from_prev = false;
            if same {
                perm = self.procs[p as usize].perm;
                if perm == 0 {
                    self.perm_epoch += 1;
                    perm = self.perm_epoch;
                    self.procs[p as usize].perm = perm;
                    from_prev = again;
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
                std::mem::swap(&mut st.sens_prev, &mut st.sens);
                st.sens.clear();
                from_prev = true;
            }
            let list = if from_prev { &st.sens_prev } else { &st.sens };
            for &(s, range) in list {
                let sig = &mut self.signals[s.0 as usize];
                sig.watched = true;
                if range != Range::new(0, sig.ty.len) {
                    sig.watch_part(range, self.cfg.cells && !self.watch_changes);
                }
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
            st.sens.clear();
            if timed {
                st.sens_prev.clear();
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
        late.clear();
        self.late = late;
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
        let which = self.polled(which, phase);
        if which == 0 && self.obs_phases.len() <= 64 {
            return;
        }
        self.notify_which(which, |o, h| {
            if o.wants_phase(phase) {
                o.phase(h, phase)
            }
        });
    }

    /// `which` without the observers that say no (`Observer::phase_due`) before a hook is made.
    #[inline(never)]
    fn polled(&mut self, mut which: u64, phase: Phase) -> u64 {
        let mut left = which;
        while left != 0 && self.obs_phases.len() <= 64 {
            let i = left.trailing_zeros() as usize;
            left &= left - 1;
            let o = &mut self.observers[i];
            if !(o.wants_phase(phase) && o.phase_due(phase)) {
                which &= !(1 << i);
            }
        }
        which
    }

    /// The changed ranges of `changed_sigs` to every observer (`Observer::value_changes`): one
    /// hook and no copy of the range lists.
    fn notify_changes(&mut self) {
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
            next: self.obs_next,
            observer: 0,
        };
        for (i, o) in observers.iter_mut().enumerate() {
            if o.wants_value_changes() {
                hook.observer = i;
                o.value_changes(&mut hook, &self.changed_sigs);
            }
        }
        self.observers = observers;
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
            next: self.obs_next,
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
    finish_applied(sig, fused, now, cycle)
}

/// `finish_signal` after `Signal::end_apply` said whether a quick transaction made an event.
fn finish_applied(sig: &mut Signal, fused: bool, now: Time, cycle: u64) -> Result<bool, ErrorKind> {
    // Direct transactions, element order: first, so the runs stay in order when nothing else
    // came.
    let pend = sig.finish_pend(now, cycle, false);
    let changed = if sig.dirty.is_empty() && sig.fast_dirty.is_empty() {
        fused
    } else {
        finish_dirty(sig, now, cycle)? | fused
    } | (pend == Some(true));
    if fused || pend.is_some() && !sig.changed.is_sorted_by_key(|r| r.offset) {
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

/// Commit the direct transactions `out` and `outv` of a process (`Ctx::assign_code`, `Out`) to
/// their signals (`Signal::commit_code`, `commit_planes`), listing each signal once in
/// `direct_sigs` for the next update. The entries of one driver are in the order they were made in
/// when all of its `out` entries come before its `outv` ones (`Ctx` keeps it so: a one-element
/// assignment after a vector entry of the same run is an `OutV` of one element); `outb` is for byte
/// drivers only.
#[inline]
fn commit_out(
    stats: &mut Stats,
    signals: &mut [Signal],
    direct_sigs: &mut Vec<u32>,
    empty: &mut bool,
    out: &mut Vec<Out>,
    outv: &mut Vec<OutV>,
    outb: &mut Vec<OutB>,
) {
    stats.direct_codes += out.len() as u64;
    stats.direct_planes += outv.len() as u64;
    stats.direct_bytes += outb.len() as u64;
    // Runs of entries of one signal (processes of one vector, in order) at once.
    let mut i = 0;
    while let Some(&o) = out.get(i) {
        let n = if out.get(i + 1).is_some_and(|x| x.sig == o.sig) {
            out[i..].iter().take_while(|x| x.sig == o.sig).count()
        } else {
            1
        };
        let sig = &mut signals[o.sig as usize];
        let first = if n == 1 {
            match sig.commit_code(o.slot, o.elem, o.code as u8) {
                Some(first) => first,
                None => {
                    *empty = true;
                    false
                }
            }
        } else {
            sig.commit_codes(&out[i..i + n], empty)
        };
        if first {
            direct_sigs.push(o.sig);
        }
        i += n;
    }
    out.clear();
    for o in outb.iter() {
        let sig = &mut signals[o.sig as usize];
        match sig.commit_bytes(o.slot, o.elem, o.es as usize, &o.val) {
            Some(true) => direct_sigs.push(o.sig),
            Some(false) => {}
            None => *empty = true,
        }
    }
    outb.clear();
    if outv.is_empty() {
        return;
    }
    let mut i = 0;
    while let Some(o) = outv.get(i) {
        let n = if outv.get(i + 1).is_some_and(|x| x.sig == o.sig) {
            outv[i..].iter().take_while(|x| x.sig == o.sig).count()
        } else {
            1
        };
        let sig = &mut signals[o.sig as usize];
        let first = if n == 1 {
            sig.commit_planes(o.slot, o.elem, o.len, o.planes)
        } else {
            sig.commit_planes_run(&outv[i..i + n])
        };
        if first {
            direct_sigs.push(o.sig);
        }
        i += n;
    }
    outv.clear();
}

/// Whether the process of trigger `t` must run in this cycle: its edge, or its reset level.
fn fires(signals: &[Signal], t: crate::EdgeTrigger, cycle: u64) -> bool {
    edge(signals, t, cycle)
        || t.reset
            .is_some_and(|(r, e, c)| signals[r.0 as usize].cur_code(e as usize) == c)
}

/// The trigger of a wait that resumes its process only on the edge `edge_key` made
/// (`Ctx::wait_edge`).
fn dyn_trigger(key: u64) -> crate::EdgeTrigger {
    crate::EdgeTrigger {
        signal: SignalId((key >> 32) as u32),
        elem: ((key & 0xffff_ffff) >> 2) as u32,
        rising: key & 1 != 0,
        legacy: key & u64::from(crate::EDGE_LEVEL) != 0,
        reset: None,
    }
}

/// Alternatives of conjunctions of literals (`IdleRead::guard`).
type Alts = Vec<Vec<crate::builder::IdleLit>>;

/// The condition of a read: one literal, or alternatives of conjunctions of literals.
enum ReaderGuard {
    One(crate::builder::IdleLit),
    Any(Alts),
}

impl ReaderGuard {
    /// Whether the condition holds now. A literal on a signal that is not logic holds (the read
    /// counts), so does one on an element out of range.
    #[inline(always)]
    fn holds(&self, signals: &[Signal]) -> bool {
        let lit = |l: &crate::builder::IdleLit| {
            let s = &signals[l.signal.0 as usize];
            !matches!(s.ty.elem, Elem::Logic)
                || l.elem >= s.ty.len
                || (s.cur_code(l.elem as usize) == l.code) == l.eq
        };
        match self {
            ReaderGuard::One(l) => lit(l),
            ReaderGuard::Any(alts) => alts.iter().any(|conj| conj.iter().all(lit)),
        }
    }
}

/// A process that reads a signal (`SimControl::readers`): the index in `reader_guards` of the
/// condition of its read (or `NO_GUARD`) and the elements it reads (the hull of its reads of the
/// signal; length `u32::MAX` for all of it). `mask` is those elements as bits when the signal
/// has at most 64 and it is a part, else 0.
#[derive(Clone, Copy)]
struct Reader {
    p: u32,
    g: u32,
    r: Range,
    mask: u64,
}

/// `SimControl::readers`: the read has no condition.
const NO_GUARD: u32 = u32::MAX;

/// The activations over which `note_idle` counts a process's skips (`ProcMeta::idle_win`).
const IDLE_WINDOW: u16 = 128;

/// A clocked process of `idle_ok` finished a plain activation (committed from its own lists): the
/// signals it reads had no event since, until one does (`armed`), and what it did is what a skipped
/// activation would do. With `Config::idle_check` an activation `idle_filter` would have skipped
/// is checked against that first.
fn note_idle(
    meta: &mut ProcMeta,
    armed: &mut bool,
    signals: &[Signal],
    name: &str,
    st: &ProcState,
) -> bool {
    // Tracking costs ~40 instructions an activation and a skip saves ~1000: a process that is
    // skipped in fewer than 3% of its activations over a window stops being tracked for a while (a
    // design whose registers always change pays for the bookkeeping and gains nothing).
    if meta.idle_state != 0 {
        meta.idle_win = meta.idle_win.saturating_add(1);
        if meta.idle_win >= IDLE_WINDOW {
            let dead = meta.idle_hits < IDLE_WINDOW / 32;
            meta.idle_win = 0;
            meta.idle_hits = 0;
            if dead {
                meta.idle_ok = false;
                meta.idle_state = 0;
                meta.shadow = false;
                return true;
            }
        }
    }
    let assigned = !(st.out.is_empty()
        && st.outv.is_empty()
        && st.outb.is_empty()
        && st.scheduled.is_empty()
        && st.phantoms.is_empty());
    if std::mem::take(&mut meta.shadow) {
        let forced = |s: u32| {
            let s = &signals[s as usize];
            s.force_eff.is_some() || s.force_drv.is_some()
        };
        let mut same = meta.idle_state == 2 || !assigned;
        // The net value of each element: a later assignment of the run (a field set after
        // `r <= r_next`) replaces an earlier one.
        let mut codes: std::collections::HashMap<(u32, u32), u8> = std::collections::HashMap::new();
        for o in &st.out {
            codes.insert((o.sig, o.elem), o.code as u8);
        }
        for o in &st.outv {
            for k in 0..o.len {
                let c = (0..4).fold(0u8, |c, p| c | ((o.planes[p] >> k & 1) as u8) << p);
                codes.insert((o.sig, o.elem + k), c);
            }
        }
        for (&(sig, elem), &c) in &codes {
            same &= forced(sig) || signals[sig as usize].cur_code(elem as usize) == c;
        }
        let mut bytes: std::collections::HashMap<(u32, u32), &crate::process::OutB> =
            std::collections::HashMap::new();
        for o in &st.outb {
            bytes.insert((o.sig, o.elem), o);
        }
        for o in bytes.values() {
            same &= forced(o.sig)
                || signals[o.sig as usize]
                    .cur
                    .bytes(o.elem as usize, 1)
                    .is_some_and(|b| b == &o.val[..o.es as usize]);
        }
        assert!(
            same,
            "idle skip: process {name} would not have done what its last activation did"
        );
    }
    meta.idle_state = 1 + u8::from(assigned);
    *armed = false;
    false
}

/// `SimControl::wake` on the fields it touches, for a caller that holds borrows of others.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn wake_proc(
    procs: &mut [ProcMeta],
    postponed: &mut Vec<u32>,
    ready_several: &mut Vec<u32>,
    ready_mono: &mut bool,
    ready: &mut Vec<u32>,
    wakes: &mut u64,
    p: u32,
    by_timeout: bool,
) {
    let meta = &mut procs[p as usize];
    if meta.ready {
        return;
    }
    meta.ready = true;
    meta.by_timeout = by_timeout;
    meta.generation += 1;
    *wakes += 1;
    if meta.flags.postponed {
        postponed.push(p);
    } else if meta.flags.several_waits {
        ready_several.push(p);
    } else {
        *ready_mono &= ready.last().is_none_or(|&l| l < p);
        ready.push(p);
    }
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
    // `To_X01` of a code: 0 for '0' and 'L', 1 for '1' and 'H', 2 (neither) for the rest.
    const X01: [u8; 16] = {
        let mut t = [2u8; 16];
        t[StdULogic::Zero as usize] = 0;
        t[StdULogic::L as usize] = 0;
        t[StdULogic::One as usize] = 1;
        t[StdULogic::H as usize] = 1;
        t
    };
    let to_x01 = |c: u8| X01[usize::from(c & 15)];
    let want = u8::from(t.rising);
    s.event_in(Range::new(t.elem, 1), cycle)
        && to_x01(s.cur_code(i)) == want
        && to_x01(s.last_value.code(i)) == 1 - want
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
/// The error of a panic in the code of process `p`: a broken invariant, reported as an
/// internal error of that process, not a bare panic.
#[cold]
fn process_panic(sh: &Shared, p: u32, panic: &(dyn std::any::Any + Send)) -> SimError {
    let msg = panic_message(panic);
    let kind = ErrorKind::Internal(format!("process panicked: {msg}"));
    let mut e = SimError::at(kind, sh.now, sh.delta);
    e.process = Some((ProcId(p), sh.names[p as usize].clone()));
    e
}

/// What the resumption of process `p` tells it, set when it runs (not when it is woken, so
/// the wake touches no slot): whether a timeout resumed it, and, for a signal that resumed a
/// process with an edge trigger, the edge that holds (`Ctx::edge`). Not for the first run.
#[inline(always)]
fn resumption(procs: &[ProcMeta], p: u32, st: &mut ProcState) {
    if let Some(m) = &mut st.marks {
        m.epoch += 1;
    }
    let by_timeout = procs[p as usize].by_timeout;
    st.act = 0;
    st.timed_out = by_timeout;
    st.known = if by_timeout {
        crate::process::NO_EDGE
    } else {
        st.known_key
    };
}

/// A batch of at least this many processes prefetches (a smaller one stays in the cache, and the
/// hint costs more than it saves), the process this many steps ahead, its first lines (the
/// start of the `Process` object: `InterpProcess` keeps its machine's hot lines there).
const PREFETCH_BATCH: usize = 128;
const PREFETCH_AHEAD: usize = 3;
const PREFETCH_LINES: usize = 5;
/// Every line a slot can touch: it need not start on a line.
const SLOT_LINES: usize = std::mem::size_of::<Mutex<Slot>>().div_ceil(yavs_prefetch::LINE) + 1;

/// A part of a concurrent copy the kernel commits itself (`SimControl::forward_copies`): source
/// elements, the direct driver (signal and source slot) and its target elements.
#[derive(Clone, Copy)]
struct Fwd {
    src: u32,
    src_off: u32,
    len: u32,
    dst: u32,
    slot: u32,
    dst_off: u32,
    /// Index of the driver in its process's `ProcState::waveforms`.
    local: u32,
    /// Bytes per element of a byte signal (one element), 0 for logic.
    es: u32,
}

/// The concurrent copies whose every part goes to a direct driver of the copy (logic, or one
/// element of a byte signal of the source's element size): `SimControl::fwd`.
fn copy_forwards(
    slots: &mut [Mutex<Slot>],
    procs: &[ProcMeta],
    signals: &[Signal],
    drivers: &[DriverInfo],
) -> Vec<Option<Box<[Fwd]>>> {
    let (mut fwd, n) = (Vec::new(), slots.len());
    for (p, slot) in slots.iter_mut().enumerate() {
        let Some(parts) = slot_mut(slot).process.copy_parts() else {
            continue;
        };
        if procs[p].flags.postponed {
            continue;
        }
        let part = |&(src, d, r): &(SigRef, DriverId, Range)| {
            let info = drivers.get(d.0 as usize)?;
            let sr = src.range?;
            let es = match (
                signals.get(src.signal.0 as usize)?.ty.elem,
                signals.get(info.signal.0 as usize)?.ty.elem,
            ) {
                (Elem::Logic, Elem::Logic) if info.direct => 0,
                (Elem::Bytes(a), Elem::Bytes(b)) if a == b && info.bdirect && r.len == 1 => a,
                _ => return None,
            };
            (info.owner.0 as usize == p && sr.len == r.len && r.len > 0 && info.range.contains(r))
                .then_some(Fwd {
                    src: src.signal.0,
                    src_off: sr.offset,
                    len: r.len,
                    dst: info.signal.0,
                    slot: info.slot,
                    dst_off: r.offset,
                    local: info.local,
                    es,
                })
        };
        if let Some(f) = parts.iter().map(part).collect::<Option<Box<[Fwd]>>>() {
            if fwd.is_empty() {
                fwd = vec![None; n];
            }
            fwd[p] = Some(f);
        }
    }
    fwd
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
