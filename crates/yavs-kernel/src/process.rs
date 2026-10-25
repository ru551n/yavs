//! The process interface the interpreter and the LLVM tier implement.

use crate::error::{ErrorKind, SimError};
use crate::signal::{
    BQuick, DelayMode, DriverId, ForceMode, Quick, Range, SigRef, Signal, SignalId, Txn, TxnList,
    VQuick, Waveform,
};
use crate::time::{Time, Unit};
use crate::value::{Buf, LogicVec, Value, ValueRef};
use smallvec::SmallVec;
use yavs_logic::StdULogic;

/// Stable id of a process: its index in elaboration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcId(pub u32);

/// `severity_level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Note,
    Warning,
    Error,
    Failure,
}

/// A VHDL process.
///
/// `run` executes from the point of the last suspension until the next `wait` statement,
/// then returns. Before returning it states what it waits for with [`Ctx::wait_on`] and
/// [`Ctx::wait_for`]/[`Ctx::wait_at`]; returning with neither waits forever (a terminated
/// process or a bare `wait;`).
///
/// A `wait until` condition belongs to the process: on resumption it evaluates the condition
/// and, when false and not [`Ctx::timed_out`], suspends again with the same sensitivity and
/// the same absolute deadline.
///
/// `run` may execute on any worker thread. It must only read signals through `ctx` and touch
/// no state shared with other processes, unless the process was built as `serial` (shared
/// variables and, through M3, every caller of a protected type).
pub trait Process: Send {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError>;

    /// The process touches state shared with other processes (shared variables, protected
    /// objects, disk files): the kernel runs it as if built with `ProcFlags::serial`.
    fn serial(&self) -> bool {
        false
    }

    /// The process suspends at more than one point: the kernel orders it as if built with
    /// `ProcFlags::several_waits`.
    fn several_waits(&self) -> bool {
        false
    }

    fn concurrent_call(&self) -> bool {
        false
    }

    /// The function an interpreted process starts in (an IR `FuncId`), for the profiler's
    /// source lines.
    fn entry_func(&self) -> Option<u32> {
        None
    }

    /// The process is a clock generator (`ToggleProcess`): its driver and the delay it assigns
    /// the complement after. The kernel then re-arms the transaction itself when it matures
    /// (`SimControl::rearm`) and leaves the process suspended.
    fn generator(&self) -> Option<(DriverId, Time)> {
        None
    }

    /// The process as `Any`, so an observer that knows its implementation can reach its
    /// state while it is suspended (`Hook::with_process`: VHPI reads and writes variables).
    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
}

impl<F: FnMut(&mut Ctx<'_>) -> Result<(), SimError> + Send> Process for F {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        self(ctx)
    }
}

/// A process whose whole body is `target <= source;` (zero delay) of signals of the same
/// structure: the kernel copies each source part to its driver and waits for the sources, with
/// no process code (the elaborator recognises the shape, #529). Parts are in element order.
pub struct CopyProcess {
    /// Source part, the driver of the matching target part, that target part.
    parts: Vec<(SigRef, DriverId, Range)>,
    buf: Vec<u8>,
}

impl CopyProcess {
    pub fn new(parts: Vec<(SigRef, DriverId, Range)>) -> CopyProcess {
        CopyProcess {
            parts,
            buf: Vec::new(),
        }
    }
}

impl Process for CopyProcess {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let ty = |ctx: &Ctx<'_>, r: SigRef| {
            ctx.sh
                .signals
                .get(r.signal.0 as usize)
                .map(|s| s.ty.elem)
                .ok_or_else(|| ctx.error(ErrorKind::BadRef(format!("signal {}", r.signal.0))))
        };
        for &(src, d, dst) in &self.parts {
            let n = src.range.map_or(0, |r| r.len) as usize;
            match ty(ctx, src)? {
                crate::value::Elem::Logic => {
                    self.buf.resize(n.div_ceil(64) * 32, 0);
                    ctx.read_into(src, &mut self.buf)?;
                    ctx.assign_blocks(d, Some(dst), Time::ZERO, &self.buf, DelayMode::INERTIAL)?;
                }
                crate::value::Elem::Bytes(es) => {
                    self.buf.resize(n * es as usize, 0);
                    ctx.read_into(src, &mut self.buf)?;
                    ctx.assign_bytes(d, Some(dst), Time::ZERO, &self.buf, DelayMode::INERTIAL)?;
                }
            }
        }
        if !ctx.wait_same() {
            for &(src, _, _) in &self.parts {
                ctx.wait_on(src)?;
            }
        }
        Ok(())
    }
}

/// A process whose whole body is `x <= not x after T;` on one scalar element of a logic
/// signal (std_ulogic, bit or boolean: a clock generator): the kernel reads the element, assigns its complement and waits for it again,
/// with no process code (the elaborator recognises the shape and evaluates `T`, #440).
pub struct ToggleProcess {
    signal: SignalId,
    elem: u32,
    /// The driver of the element and the element as a range of it.
    driver: DriverId,
    range: Range,
    delay: Time,
    /// `delay` truncated to the resolution, once known (`None`: not yet).
    quantized: Option<Time>,
    mode: DelayMode,
}

impl ToggleProcess {
    pub fn new(
        signal: SignalId,
        elem: u32,
        driver: DriverId,
        delay: Time,
        mode: DelayMode,
    ) -> ToggleProcess {
        ToggleProcess {
            signal,
            elem,
            driver,
            range: Range::new(elem, 1),
            delay,
            quantized: None,
            mode,
        }
    }
}

impl Process for ToggleProcess {
    fn generator(&self) -> Option<(DriverId, Time)> {
        Some((self.driver, self.delay))
    }

    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let c = ctx.elem_code(self.signal, self.elem)?;
        let n = StdULogic::from_code_pub(c).not() as u8;
        let q = *self
            .quantized
            .get_or_insert_with(|| self.delay.quantize(ctx.sh.resolution));
        // A positive delay to an idle driver: one quick transaction, nothing to preempt.
        let at = ctx.sh.now.checked_add(q).ok().filter(|_| q != Time::ZERO);
        let fast = at.is_some_and(|at| ctx.assign_later(self.driver, self.elem, at, n));
        if !fast {
            ctx.assign_code(self.driver, Some(self.range), self.delay, n, self.mode)?;
        }
        if !ctx.wait_on_again(self.signal, self.range) {
            ctx.wait_on(self.signal.slice(self.elem, 1))?;
        }
        Ok(())
    }
}

/// Writes reports and textio `OUTPUT` text as a serially running process makes them, in the
/// order they would be committed in (#483). An observer offers one with
/// [`Observer::live_sink`](crate::Observer::live_sink) and then gets such reports only once,
/// here, not through `report`/`output`.
pub trait LiveSink: Send + Sync {
    fn report(&self, now: Time, delta: u32, process: &str, severity: Severity, message: &str);
    fn output(&self, bytes: &[u8]);
}

/// A buffered side effect, committed after the delta in elaboration order. The flag on a
/// report or output: already written to the live sink.
pub(crate) enum Effect {
    Report(Severity, String, bool),
    Output(Vec<u8>, bool),
    Stop(i32),
    Finish(i32),
    Op(SignalOp),
}

/// A force, release or deposit, applied in the next simulation cycle.
pub(crate) struct SignalOp {
    pub signal: SignalId,
    pub range: Range,
    pub mode: OpMode,
    /// The value for `Force` and `Deposit`.
    pub value: Option<Buf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpMode {
    Force(ForceMode),
    Release(ForceMode),
    Deposit,
}

/// A zero-delay assignment of one element to a direct driver (`DriverInfo::direct`) made in
/// this run, its `std_ulogic` code: the commit writes it to the signal (`commit_out`) instead
/// of queueing a transaction. A later assignment to the same driver in the run is another
/// entry (the last one wins, as zero-delay preemption gives); one that needs the waveform
/// moves the driver's entries there first (`Ctx::spill_out`). The entries of one driver are
/// all in `ProcState::out` or all in `outv`, so their order is kept.
#[derive(Clone, Copy)]
pub(crate) struct Out {
    pub sig: u32,
    pub slot: u32,
    pub elem: u32,
    pub code: u32,
}

/// `Out` of at most 64 elements, as plane words.
#[derive(Clone, Copy)]
pub(crate) struct OutV {
    pub sig: u32,
    pub slot: u32,
    pub elem: u32,
    pub len: u32,
    pub planes: [u64; 4],
}

/// `Out` of one element of a byte signal of at most 8 bytes per element (`DriverInfo::bdirect`):
/// its bytes. The commit (`Signal::commit_bytes`) writes the driving value, the next update
/// finishes it (`Signal::finish_pend`).
#[derive(Clone, Copy)]
pub(crate) struct OutB {
    pub sig: u32,
    pub slot: u32,
    pub elem: u32,
    pub es: u32,
    pub val: [u8; 8],
}

/// A process with more drivers than this keeps `ProcState::marks`.
pub(crate) const MARKED_DRIVERS: usize = 4;

/// The direct-entry marks of a process with many drivers (`ProcState::marks`): the run number
/// (from 1, bumped at every resumption) and per driver (local index) `epoch << 2 | kind`.
pub(crate) struct Marks {
    pub epoch: u64,
    pub d: Vec<u64>,
}

/// Per-process state the process mutates while it runs.
#[derive(Default)]
pub(crate) struct ProcState {
    pub waveforms: Vec<Waveform>,
    pub effects: Vec<Effect>,
    pub sens: Vec<(SignalId, Range)>,
    /// The sensitivity the last commit registered, when that wait had no timeout.
    pub sens_prev: Vec<(SignalId, Range)>,
    /// The coming wait is exactly the one `sens_prev` holds (`Ctx::wait_on_again`): `sens`
    /// stays empty and the commit takes it from `sens_prev`.
    pub again: bool,
    /// The leading ranges of `sens_prev` this wait named again so far with `sens` empty
    /// (`Ctx::wait_on_again`); all of them when `again`. The commit takes a part from
    /// `sens_prev` ahead of the ranges named after it.
    pub matched: u32,
    pub deadline: Option<Time>,
    pub timed_out: bool,
    /// Times of transactions scheduled during this run.
    pub scheduled: Vec<(Time, DriverId)>,
    /// Drivers (locals) that hold skipped unchanged assignments (`Waveform::phantom`) from
    /// this run.
    pub phantoms: Vec<u32>,
    /// The edge of the process's trigger the kernel resumes it for (`Builder::edge_trigger`:
    /// signal, element, kind; set once by `SimControl::new`) and its `edge_key` (`NO_EDGE`
    /// without one). `known` is that key for an activation a signal resumed (not a timeout or
    /// the initial run), else `NO_EDGE`: the edge then holds for the whole activation, so
    /// `Ctx::edge` of it need not look.
    pub known_edge: Option<(SignalId, u32, u8)>,
    pub known_key: u64,
    pub known: u64,
    /// The edge (`edge_key`) the coming wait resumes the process for (`Ctx::wait_edge`),
    /// `NO_EDGE` for none; the commit takes it and puts `NO_EDGE` back.
    pub wait_key: u64,
    /// The run's number (from 1, bumped at every resumption), and per driver (local index) which
    /// kind of direct entry it has in this run: `epoch << 2 | kind`, kind 1 `out`, 2 `outv`,
    /// 3 `outb`. A mark of an older run means none, so the entries need no scan and no clearing.
    /// Only for a process of more than `MARKED_DRIVERS` drivers (empty otherwise: the lists of a
    /// small process are short, scanning them is cheaper than keeping marks).
    pub marks: Option<Box<Marks>>,
    /// A one-element logic value kept for `Ctx::assign_code`.
    pub one: Option<LogicVec>,
    /// Direct transactions of this run: one element, and up to 64.
    pub out: Vec<Out>,
    pub outv: Vec<OutV>,
    pub outb: Vec<OutB>,
    /// What this run did, `ACT_*` bits; cleared at every resumption (`kernel::resumption`). The
    /// commit looks at the lists only when a bit says they may hold something.
    pub act: u8,
}

/// `ProcState::act`: the run set a timeout or made an effect (`deadline`, `effects`), so the
/// commit is not the usual clocked one.
pub(crate) const ACT_SLOW: u8 = 1;
/// `ProcState::act`: the run scheduled a transaction or made a direct entry or a phantom
/// (`scheduled`, `phantoms`, `out`, `outv`, `outb`). Every push sets it.
pub(crate) const ACT_ASSIGNED: u8 = 2;

/// `ProcState::known` when no edge is known.
pub(crate) const NO_EDGE: u64 = u64::MAX;

/// One word naming an edge (signal, element, kind) for the known-edge test. An element is below
/// 2^30 (a signal that large does not fit in memory); `Ctx::edge_known` of a larger one cannot
/// match a key that `SimControl::new` made.
#[inline(always)]
pub fn edge_key(signal: SignalId, elem: u32, kind: u8) -> u64 {
    u64::from(signal.0) << 32 | u64::from(elem) << 2 | u64::from(kind)
}

/// Added to the kind of an edge (`known_edge`, `Ctx::edge_to`) that is an event ending on the
/// level (`x'event and x = '1'`), not the IEEE function.
pub const EDGE_LEVEL: u8 = 2;

/// In `ProcState::phantoms`: a skipped assignment to a direct driver (no waveform entry).
const SKIPPED: u32 = u32::MAX;

impl ProcState {
    /// The summary flags say what the lists hold (a flag may be set with its lists empty, not the
    /// other way round).
    #[inline(always)]
    pub fn check_flags(&self) {
        debug_assert!(
            self.act & ACT_SLOW != 0 || (self.deadline.is_none() && self.effects.is_empty()),
            "ACT_SLOW missing"
        );
        debug_assert!(
            self.act & ACT_ASSIGNED != 0
                || (self.scheduled.is_empty()
                    && self.phantoms.is_empty()
                    && self.out.is_empty()
                    && self.outv.is_empty()
                    && self.outb.is_empty()),
            "ACT_ASSIGNED missing"
        );
        debug_assert!(!self.again || self.sens.is_empty(), "again with sens");
    }

    /// End of a run: whether skipped assignments are left, which the kernel must replace by an
    /// empty delta.
    pub fn take_phantoms(&mut self) -> bool {
        let mut any = false;
        for l in self.phantoms.drain(..) {
            if l == SKIPPED {
                any = true;
                continue;
            }
            let p = &mut self.waveforms[l as usize].phantom;
            any |= !p.is_empty();
            p.clear();
        }
        any
    }
}

pub(crate) struct DriverInfo {
    pub owner: ProcId,
    /// Index into the owner's `ProcState::waveforms`.
    pub local: u32,
    pub signal: SignalId,
    pub range: Range,
    /// Index into the signal's source list.
    pub slot: u32,
    /// The driver is the only source of its elements of a logic signal with no port, group,
    /// guard or resolution function (`SimControl::new`): its zero-delay quick transactions go
    /// straight to the signal at commit (`SimControl::direct`), not through the queue.
    pub direct: bool,
    /// The driver is the only source of its elements of a byte signal (an enumeration, an
    /// integer, a boolean, a real, a time of at most 8 bytes) with no port, group, guard or
    /// resolution function: its zero-delay one-element assignments are `OutB` entries
    /// committed like `Out` ones.
    pub bdirect: bool,
}

/// Read-only kernel state shared by all processes running in a delta.
pub(crate) struct Shared<'a> {
    pub now: Time,
    pub delta: u32,
    pub cycle: u64,
    pub resolution: Unit,
    pub stop_severity: Severity,
    pub signals: &'a [Signal],
    pub drivers: &'a [DriverInfo],
    pub names: &'a [String],
    /// Where a live process (`Ctx::live`) writes its reports.
    pub live: Option<&'a dyn LiveSink>,
    /// `Interrupt` of the simulation (`Ctx::stop_requested`).
    pub interrupt: &'a std::sync::atomic::AtomicU8,
}

/// What a running process sees: current-cycle signal values, its own drivers and its own
/// effect buffer.
pub struct Ctx<'a> {
    pub(crate) sh: &'a Shared<'a>,
    pub(crate) pid: ProcId,
    pub(crate) st: &'a mut ProcState,
    /// The process runs where its reports may go straight to `Shared::live`: nothing that
    /// commits before them is still unwritten.
    pub(crate) live: bool,
}

impl Ctx<'_> {
    #[inline]
    pub fn now(&self) -> Time {
        self.sh.now
    }

    #[inline]
    pub fn delta(&self) -> u32 {
        self.sh.delta
    }

    /// `Config::stop_severity`: a report at this severity ends the simulation after the
    /// current cycle, so the process should not execute past it.
    #[inline]
    pub fn stop_severity(&self) -> Severity {
        self.sh.stop_severity
    }

    #[inline]
    pub fn process(&self) -> ProcId {
        self.pid
    }

    /// `Interrupt::stop` was called (Ctrl-C): a process that never suspends polls it at the
    /// jump back of its loop, because the kernel only looks between cycles.
    #[inline]
    pub fn stop_requested(&self) -> bool {
        crate::kernel::Interrupt::stopped(self.sh.interrupt)
    }

    /// The flag byte `stop_requested` reads (`Interrupt::STOP` set: stop), for compiled code that
    /// polls it itself. Valid for the whole run.
    #[inline]
    pub fn interrupt_flags(&self) -> *const u8 {
        self.sh.interrupt.as_ptr()
    }

    /// An error with this process, time and delta attached. The interpreter adds its call
    /// stack to `trace`.
    pub fn error(&self, kind: ErrorKind) -> SimError {
        let mut e = SimError::at(kind, self.sh.now, self.sh.delta);
        e.process = Some((self.pid, self.sh.names[self.pid.0 as usize].clone()));
        e
    }

    #[inline(always)]
    fn sig(&self, r: impl Into<SigRef>) -> Result<(&Signal, Range), SimError> {
        check_ref(self.sh.signals, r.into()).map_err(|k| self.error(k))
    }

    /// Current value of a byte signal.
    #[inline]
    pub fn value(&self, r: impl Into<SigRef>) -> Result<&[u8], SimError> {
        let (s, r) = self.sig(r)?;
        s.cur
            .bytes(r.offset as usize, r.len as usize)
            .ok_or_else(|| {
                self.error(ErrorKind::BadRef(format!(
                    "{} holds logic values, read it with logic()",
                    s.name
                )))
            })
    }

    /// Current value of a logic signal.
    pub fn logic(&self, r: impl Into<SigRef>) -> Result<LogicVec, SimError> {
        let (s, r) = self.sig(r)?;
        match s.cur.extract(r.offset as usize, r.len as usize).to_value() {
            Value::Logic(l) => Ok(l),
            Value::Bytes(_) => Err(self.error(ErrorKind::BadRef(format!(
                "{} holds bytes, read it with value()",
                s.name
            )))),
        }
    }

    /// Current value of `r` copied into `out` without allocating: the bytes of a byte signal,
    /// or the bit-plane blocks of a logic signal (four little-endian `u64` planes per 64
    /// elements, unused bits zero, as `LogicVec`'s blocks). `out` must be exactly that size.
    #[inline(always)]
    pub fn read_into(&self, r: impl Into<SigRef>, out: &mut [u8]) -> Result<(), SimError> {
        let (s, r) = self.sig(r)?;
        // A logic value of one word (a register read): copied here, no call.
        if let (Buf::Planes { b, .. }, Ok(dst)) = (&s.cur, <&mut [u8; 32]>::try_from(&mut *out))
            && (1..=64).contains(&r.len)
        {
            let w = yavs_logic::wide::get_block(b, r.offset as usize, r.len as usize);
            for (d, x) in dst.as_chunks_mut::<8>().0.iter_mut().zip(w) {
                *d = x.to_le_bytes();
            }
            return Ok(());
        }
        self.read_slow(s, r, out)
    }

    #[inline(never)]
    fn read_slow(&self, s: &Signal, r: Range, out: &mut [u8]) -> Result<(), SimError> {
        if s.cur.write_to(r.offset as usize, r.len as usize, out) {
            return Ok(());
        }
        Err(self.error(ErrorKind::BadRef(format!(
            "{}: a {}-byte buffer for {} elements",
            s.name,
            out.len(),
            r.len
        ))))
    }

    /// `'event`: an event on any element in this cycle.
    #[inline]
    pub fn event(&self, r: impl Into<SigRef>) -> Result<bool, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.event_in(r, self.sh.cycle))
    }

    /// `'active`.
    pub fn active(&self, r: impl Into<SigRef>) -> Result<bool, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.active_in(r, self.sh.cycle))
    }

    /// `'last_event`: time since the most recent event, `TIME'HIGH` if none.
    pub fn last_event(&self, r: impl Into<SigRef>) -> Result<Time, SimError> {
        let (s, r) = self.sig(r)?;
        self.history(s, r, false)?;
        Ok(s.last_event(r, self.sh.now))
    }

    /// A lean signal (`Builder::lean_events`) cannot answer `'last_event` or `'last_value` of a
    /// range that is not whole blocks: the elaborator found no such use, so this is a bug.
    fn history(&self, s: &Signal, r: Range, last_value: bool) -> Result<(), SimError> {
        if s.history_exact(r, last_value) {
            return Ok(());
        }
        Err(self.error(ErrorKind::Internal(format!(
            "'{} of elements {}..{} of signal {}, which keeps only each block's last event \
             (this is a bug in yavs: the use was not found at elaboration)",
            if last_value {
                "last_value"
            } else {
                "last_event"
            },
            r.offset,
            r.end(),
            s.name
        ))))
    }

    /// `'last_active`.
    pub fn last_active(&self, r: impl Into<SigRef>) -> Result<Time, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.last_active(r, self.sh.now))
    }

    /// `'last_value` of the range as a whole.
    pub fn last_value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = self.sig(r)?;
        self.history(s, r, true)?;
        Ok(s.last_value_of(r).to_value())
    }

    /// `'last_value` of one element of a logic signal as its `std_ulogic` code, without building
    /// a value (the element's own history is all there is to a single element, so this is
    /// [`Ctx::last_value`]); `None` where that does not apply (more or fewer elements, a signal
    /// that holds bytes), and the caller reads the value.
    #[inline]
    pub fn last_code(&self, r: impl Into<SigRef>) -> Result<Option<u8>, SimError> {
        let (s, r) = self.sig(r)?;
        if r.len != 1 {
            return Ok(None);
        }
        self.history(s, r, true)?;
        Ok(s.last_value.planes_code(r.offset as usize))
    }

    /// The first element of `r` of a logic signal as a `std_ulogic` code, its current value or
    /// (`last`) its `'last_value`, without allocating; `None` for a null range.
    #[inline(always)]
    pub fn first_code(&self, r: impl Into<SigRef>, last: bool) -> Result<Option<u8>, SimError> {
        let (s, r) = self.sig(r)?;
        if r.len == 0 {
            return Ok(None);
        }
        if !last && let Some(c) = s.mirrored_code(r.offset as usize) {
            return Ok(Some(c));
        }
        let b = if last { &s.last_value } else { &s.cur };
        match b.planes_code(r.offset as usize) {
            Some(c) => Ok(Some(c)),
            None => Err(self.error(ErrorKind::BadRef(format!(
                "{} holds bytes, read it with value()",
                s.name
            )))),
        }
    }

    /// The current `std_ulogic` code of element `e` of logic signal `s`: `first_code` of a
    /// one-element reference with the checks of a lone element.
    #[inline(always)]
    pub fn elem_code(&self, s: SignalId, e: u32) -> Result<u8, SimError> {
        if let Some(sig) = self.sh.signals.get(s.0 as usize)
            && let Some(c) = sig.mirrored_code(e as usize)
        {
            return Ok(c);
        }
        // The error cases (a bad reference, a byte signal) are the general path's.
        match self.first_code(s.slice(e, 1), false)? {
            Some(c) => Ok(c),
            None => Err(self.error(ErrorKind::BadRef("null range".into()))),
        }
    }

    /// The bytes of the current value of element `e` of byte signal `s`; `None` for a logic
    /// signal or an element out of range (the general `value` reports those).
    #[inline(always)]
    pub fn elem_bytes(&self, s: SignalId, e: u32) -> Option<&[u8]> {
        let sig = self.sh.signals.get(s.0 as usize)?;
        if e >= sig.ty.len {
            return None;
        }
        sig.cur.bytes(e as usize, 1)
    }

    /// The bytes of the `std_ulogic` codes of the current value of logic signal `s`, one per
    /// element, as a pointer and the number of elements, for compiled code that reads them
    /// itself. `None` for a signal that keeps no codes (bytes, over `CODES_MAX` elements, or a bus: `CODES_SOURCE_MAX`).
    /// The pointer stays valid for the whole simulation; the bytes change only in the update
    /// phase, while no process runs.
    #[inline]
    pub fn code_view(&self, s: SignalId) -> Option<(*const u8, usize)> {
        let sig = self.sh.signals.get(s.0 as usize)?;
        sig.codes_view()
    }

    /// The event stamps of signal `s` (`Signal::event_view`) for `'event` and `rising_edge` of
    /// compiled code.
    #[inline]
    pub fn event_view(&self, s: SignalId) -> Option<*const u64> {
        self.sh.signals.get(s.0 as usize)?.event_view()
    }

    /// The plane blocks of the current value of signal `s` and its element count
    /// (`Signal::planes_view`).
    #[inline]
    pub fn planes_view(&self, s: SignalId) -> Option<(*const yavs_logic::Block, usize)> {
        self.sh.signals.get(s.0 as usize)?.planes_view()
    }

    /// The plane blocks of the current value of logic signal `s` and its element count: whole
    /// record packs are read from them a block at a time (#686).
    #[inline]
    pub fn planes(&self, s: SignalId) -> Option<(&[yavs_logic::Block], u32)> {
        match &self.sh.signals.get(s.0 as usize)?.cur {
            crate::value::Buf::Planes { len, b } => Some((b, *len as u32)),
            _ => None,
        }
    }

    /// The bytes of the current value of signal `s`, the element size and the element count
    /// (`Signal::bytes_view`).
    #[inline]
    pub fn bytes_view(&self, s: SignalId) -> Option<(*const u8, usize, usize)> {
        self.sh.signals.get(s.0 as usize)?.bytes_view()
    }

    /// The plane block of the `'last_value` of signal `s` (`Signal::last_view`).
    #[inline]
    pub fn last_view(&self, s: SignalId) -> Option<*const yavs_logic::Block> {
        self.sh.signals.get(s.0 as usize)?.last_view()
    }

    /// The number of the current simulation cycle (an event stamp equal to it is an event now).
    #[inline]
    pub fn cycle(&self) -> u64 {
        self.sh.cycle
    }

    /// The bytes of `code_view(s)` that may be read (`Signal::codes_cap`), 0 without codes.
    #[inline]
    pub fn code_cap(&self, s: SignalId) -> usize {
        self.sh
            .signals
            .get(s.0 as usize)
            .map_or(0, |sig| sig.codes_cap())
    }

    /// The edge the kernel resumed this process for in this activation as an `edge_key`
    /// (`u64::MAX`: none).
    #[inline]
    pub fn resumed_edge_key(&self) -> u64 {
        self.st.known
    }

    /// The edge the kernel resumed this process for in this activation (signal, element,
    /// kind: `rising`, plus [`EDGE_LEVEL`] for an event ending on the level), if `Ctx::edge`
    /// or `Ctx::edge_by` of that kind takes it as held.
    #[inline(always)]
    pub fn known_edge(&self) -> Option<(SignalId, u32, u8)> {
        self.st.known_edge.filter(|_| self.st.known != NO_EDGE)
    }

    /// `rising_edge` / `falling_edge` of the first element of a logic signal range: an event
    /// in this cycle, `To_X01` of the current value the new level and `To_X01` of the
    /// `'last_value` the old one; `None` for a null range. One reference check instead of
    /// the three of `event` and two `first_code`.
    #[inline(always)]
    pub fn edge(&self, r: impl Into<SigRef>, rising: bool) -> Result<Option<bool>, SimError> {
        self.edge_kind(r.into(), u8::from(rising))
    }

    /// The edge test `code` of the first element of a logic signal range: bit 0 rising, plus
    /// [`EDGE_LEVEL`] for `x'event and x = '1'` / `'0'` (an event in this cycle and that level
    /// now, whatever it was before; also BIT and BOOLEAN) instead of the IEEE function
    /// ([`Ctx::edge`]). `None` for a null range.
    #[inline(always)]
    pub fn edge_by(&self, r: impl Into<SigRef>, code: u8) -> Result<Option<bool>, SimError> {
        self.edge_kind(r.into(), code)
    }

    /// Whether the kernel resumed this process for the edge `kind` of element `elem` of
    /// `signal` (`known_edge`): then it holds.
    #[inline(always)]
    pub fn edge_known(&self, signal: SignalId, elem: u32, kind: u8) -> bool {
        self.st.known == edge_key(signal, elem, kind)
    }

    #[inline(always)]
    fn edge_kind(&self, r: SigRef, kind: u8) -> Result<Option<bool>, SimError> {
        if let Some(x) = r.range
            && x.len == 1
            && self.edge_known(r.signal, x.offset, kind)
        {
            return Ok(Some(true));
        }
        self.edge_slow(r, kind)
    }

    #[inline(never)]
    fn edge_slow(&self, r: SigRef, kind: u8) -> Result<Option<bool>, SimError> {
        let (s, r) = self.sig(r)?;
        if r.len == 0 {
            return Ok(None);
        }
        let i = r.offset as usize;
        if !s.event_in(Range::new(r.offset, 1), self.sh.cycle) {
            return Ok(Some(false));
        }
        if matches!(s.cur, Buf::Bytes { .. }) {
            return Err(self.error(ErrorKind::BadRef(format!(
                "{} holds bytes, read it with value()",
                s.name
            ))));
        }
        let rising = kind & 1 != 0;
        if kind & EDGE_LEVEL != 0 {
            let end = if rising {
                StdULogic::One
            } else {
                StdULogic::Zero
            };
            return Ok(Some(s.cur_code(i) == end as u8));
        }
        let x01 = |c: u8| match c {
            c if c == StdULogic::One as u8 || c == StdULogic::H as u8 => Some(true),
            c if c == StdULogic::Zero as u8 || c == StdULogic::L as u8 => Some(false),
            _ => None,
        };
        Ok(Some(
            x01(s.cur_code(i)) == Some(rising) && x01(s.last_value.code(i)) == Some(!rising),
        ))
    }

    fn own(&self, d: DriverId) -> Result<&DriverInfo, SimError> {
        let fail = |m: &str| self.error(ErrorKind::InvalidAssign(m.into()));
        let info = self
            .sh
            .drivers
            .get(d.0 as usize)
            .ok_or_else(|| fail("no such driver"))?;
        if info.owner != self.pid {
            return Err(fail("driver belongs to another process"));
        }
        Ok(info)
    }

    /// Move this run's direct transactions of driver `d` (`Out`) into its waveform, in order,
    /// for an assignment that preempts or inspects pending transactions.
    pub(crate) fn spill_out(&mut self, d: DriverId) {
        if self.st.out.is_empty() && self.st.outv.is_empty() && self.st.outb.is_empty() {
            return;
        }
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return;
        };
        let key = (info.signal.0, info.slot);
        if self.st.marks.is_some() {
            if !self.direct_entries(info) {
                return;
            }
            // All of the driver's entries move to its waveform now.
            if let Some(m) = &mut self.st.marks {
                m.d[info.local as usize] = 0;
            }
        }
        // The entries of a byte driver (all of its entries are in `outb`).
        if self.st.outb.iter().any(|o| (o.sig, o.slot) == key) {
            let (local, now) = (info.local as usize, self.sh.now);
            let mut moved: SmallVec<[(Range, u32, [u8; 8]); 2]> = SmallVec::new();
            self.st.outb.retain(|o| {
                let hit = (o.sig, o.slot) == key;
                if hit {
                    moved.push((Range::new(o.elem, 1), o.es, o.val));
                }
                !hit
            });
            for (r, es, val) in moved {
                let value = Buf::Bytes {
                    es: es as usize,
                    data: smallvec::SmallVec::from_slice(&val[..es as usize]),
                };
                let mut t = TxnList::new();
                t.push(Txn {
                    time: now,
                    range: r,
                    value: Some(value),
                });
                self.st.waveforms[local].schedule(r, None, &mut t);
                self.st.act |= ACT_ASSIGNED;
                self.st.scheduled.push((now, d));
            }
            return;
        }
        let mut moved: SmallVec<[(Range, [u64; 4]); 2]> = SmallVec::new();
        self.st.out.retain(|o| {
            let c = u64::from(o.code);
            let hit = (o.sig, o.slot) == key;
            if hit {
                let planes = [c & 1, c >> 1 & 1, c >> 2 & 1, c >> 3 & 1];
                moved.push((Range::new(o.elem, 1), planes));
            }
            !hit
        });
        self.st.outv.retain(|o| {
            let hit = (o.sig, o.slot) == key;
            if hit {
                moved.push((Range::new(o.elem, o.len), o.planes));
            }
            !hit
        });
        let (local, now) = (info.local as usize, self.sh.now);
        for (r, planes) in moved {
            let value = Buf::Planes {
                len: r.len as usize,
                b: smallvec::smallvec![planes],
            };
            let mut t = TxnList::new();
            t.push(Txn {
                time: now,
                range: r,
                value: Some(value),
            });
            // Transport: a transaction at `now` replaces the earlier ones at `now`.
            self.st.waveforms[local].schedule(r, None, &mut t);
            self.st.act |= ACT_ASSIGNED;
            self.st.scheduled.push((now, d));
        }
    }

    /// `'driving`: false if any element of this process's driver is disconnected.
    #[inline]
    pub fn driving(&self, d: DriverId) -> Result<bool, SimError> {
        let info = self.own(d)?;
        let slot = &self.sh.signals[info.signal.0 as usize].drivers[info.slot as usize];
        Ok(slot
            .off
            .as_ref()
            .is_none_or(|off| off.iter().all(|&w| w == 0)))
    }

    /// `'driving_value`: the current value of this process's driver. An error while the driver
    /// is disconnected (LRM 16.2.4).
    pub fn driving_value(&self, d: DriverId) -> Result<Value, SimError> {
        if !self.driving(d)? {
            return Err(self.error(ErrorKind::Runtime(
                "'driving_value of a disconnected driver".into(),
            )));
        }
        let info = self.own(d)?;
        let s = &self.sh.signals[info.signal.0 as usize];
        // A direct driver whose commits wrote only the driving value (`Signal::lag`).
        if s.lag && (info.direct || info.bdirect) {
            let r = info.range;
            return Ok(s
                .driving
                .extract(r.offset as usize, r.len as usize)
                .to_value());
        }
        Ok(s.drivers[info.slot as usize].value.to_value())
    }

    /// [`Ctx::driving_value`] of elements `target` of driver `d` (the whole driver when `None`).
    pub fn driving_value_of(&self, d: DriverId, target: Option<Range>) -> Result<Value, SimError> {
        let v = self.driving_value(d)?;
        let (_, r) = self.target(d, target)?;
        let info = self.own(d)?;
        if r == info.range {
            return Ok(v);
        }
        let (at, n) = ((r.offset - info.range.offset) as usize, r.len as usize);
        Ok(match v {
            Value::Logic(l) => Value::Logic(l.slice(at, n)),
            Value::Bytes(b) => {
                let es = b.len() / info.range.len.max(1) as usize;
                Value::Bytes(b[at * es..(at + n) * es].to_vec())
            }
        })
    }

    /// `now + delay` after truncating `delay` to the resolution limit.
    pub fn after(&self, delay: Time) -> Result<Time, SimError> {
        let delay = delay.quantize(self.sh.resolution);
        let base = self.sh.now;
        base.checked_add(delay)
            .map_err(|_| self.error(ErrorKind::TimeOverflow { base, delay }))
    }

    /// Signal assignment through driver `d` to elements `target` (the whole driver when
    /// `None`). `waveform` holds `(delay, value)` pairs in ascending delay order, each value
    /// covering `target`.
    pub fn assign<'v, V: Into<ValueRef<'v>> + Copy>(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        waveform: &[(Time, V)],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        if let [(delay, v)] = waveform
            && let ValueRef::Logic(l) = (*v).into()
            && let [p] = l.repr().blocks()
            && self.assign_planes_fast(d, target, *delay, *p, Some(l.len() as u32), mode)
        {
            return Ok(());
        }
        self.spill_out(d);
        let (sig, r) = self.target(d, target)?;
        let elem = self.sh.signals[sig].ty.elem;
        let local = self.own(d)?.local as usize;
        if let [(delay, v)] = waveform
            && *delay == Time::ZERO
            && match mode {
                DelayMode::Transport => true,
                DelayMode::Inertial { reject } => reject.is_none_or(|t| t == Time::ZERO),
            }
            && self.unchanged(sig, d, r, (*v).into())
        {
            let w = &mut self.st.waveforms[local];
            if w.phantom.is_empty() {
                self.st.act |= ACT_ASSIGNED;
                self.st.phantoms.push(local as u32);
            }
            w.phantom.push(r);
            return Ok(());
        }
        // Zero delay, nothing pending or skipped on the driver: nothing to preempt, so the
        // transaction is appended as it is.
        if let [(delay, v)] = waveform
            && *delay == Time::ZERO
            && r.len > 0
            && match mode {
                DelayMode::Transport => true,
                DelayMode::Inertial { reject } => reject.is_none_or(|t| t == Time::ZERO),
            }
            && self.st.waveforms[local].idle()
            && self.st.waveforms[local].phantom.is_empty()
        {
            let n = r.len as usize;
            let buf = match self.st.waveforms[local].spare.pop() {
                Some(mut b) => b.refill(elem, n, (*v).into()).map(|()| b),
                None => Buf::from_ref(elem, n, (*v).into()),
            }
            .map_err(|m| self.error(ErrorKind::InvalidAssign(m)))?;
            let now = self.sh.now;
            self.st.waveforms[local].pending.push(Txn {
                time: now,
                range: r,
                value: Some(buf),
            });
            self.st.act |= ACT_ASSIGNED;
            self.st.scheduled.push((now, d));
            return Ok(());
        }
        let mut new = TxnList::new();
        for &(delay, v) in waveform {
            let (n, v) = (r.len as usize, v.into());
            let buf = match self.st.waveforms[local].spare.pop() {
                Some(mut b) => b.refill(elem, n, v).map(|()| b),
                None => Buf::from_ref(elem, n, v),
            }
            .map_err(|m| self.error(ErrorKind::InvalidAssign(m)))?;
            self.push_txn(&mut new, r, delay, Some(buf))?;
        }
        self.schedule(d, r, &mut new, mode)
    }

    /// `assign` of one `std_ulogic` code (`std_ulogic'pos`) with one waveform element: the
    /// scalar case without building a value or a waveform list.
    #[inline]
    pub fn assign_code(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        code: u8,
        mode: DelayMode,
    ) -> Result<(), SimError> {
        if self.assign_code_fast(d, target, delay, code, mode) {
            return Ok(());
        }
        self.assign_code_slow(d, target, delay, code, mode)
    }

    #[inline(never)]
    fn assign_code_slow(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        code: u8,
        mode: DelayMode,
    ) -> Result<(), SimError> {
        let mut one = self
            .st
            .one
            .take()
            .unwrap_or_else(|| LogicVec::new(1, StdULogic::U));
        one.set(0, StdULogic::from_code_pub(code));
        let r = self.assign(d, target, &[(delay, &one)], mode);
        self.st.one = Some(one);
        r
    }

    /// The common scalar case of `assign_code` in one pass: a zero-delay assignment of one
    /// element to a driver with nothing pending (and nothing skipped) has nothing to preempt,
    /// so the transaction is appended as it is. False when the general path must run (it also
    /// produces every error).
    #[inline(always)]
    fn assign_code_fast(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        code: u8,
        mode: DelayMode,
    ) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        let r = target.unwrap_or(info.range);
        let at = self.fast_time(delay, mode);
        if at.is_none()
            || code >= 9
            || info.owner != self.pid
            || r.len != 1
            || !info.range.contains(r)
        {
            return false;
        }
        let w = &mut self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        let now = self.sh.now;
        let at = at.unwrap_or(now);
        if at > now {
            // A delayed assignment to a driver with nothing pending has nothing to preempt.
            if (info.direct && self.direct_entries(info))
                || !matches!(
                    self.sh.signals[info.signal.0 as usize].ty.elem,
                    crate::value::Elem::Logic
                )
            {
                return false;
            }
            let w = &mut self.st.waveforms[info.local as usize];
            w.quick = Some(Quick {
                time: at,
                elem: r.offset,
                code,
            });
            self.st.act |= ACT_ASSIGNED;
            self.st.scheduled.push((at, d));
            return true;
        }
        if info.direct {
            let (sig, slot) = (info.signal.0, info.slot);
            // The driver has vector entries in this run (a record assigned whole, then one of its
            // fields): the commit applies `out` before `outv`, so the element joins the vector
            // entries, in order, as an entry of one element. No skip: the entries before it
            // decide what the signal holds.
            if self.has_entries(info, 2) {
                let c = u64::from(code);
                self.st.act |= ACT_ASSIGNED;
                self.st.outv.push(OutV {
                    sig,
                    slot,
                    elem: r.offset,
                    len: 1,
                    planes: [c & 1, c >> 1 & 1, c >> 2 & 1, c >> 3 & 1],
                });
                return true;
            }
            // The signal and the driver have the code and nothing of this run is pending: the
            // commit would find no change (`Signal::commit_code`), so the entry is not made.
            if !self.has_entries(info, 1)
                && self.sh.signals[sig as usize].direct_same(slot, r.offset, code)
            {
                self.note_skipped();
                return true;
            }
            self.mark_entries(info, 1);
            self.st.act |= ACT_ASSIGNED;
            self.st.out.push(Out {
                sig,
                slot,
                elem: r.offset,
                code: u32::from(code),
            });
            return true;
        }
        let s = &self.sh.signals[info.signal.0 as usize];
        if !matches!(s.ty.elem, crate::value::Elem::Logic) {
            return false;
        }
        // The signal's own code array is dense and hot: ask it first, the driver slot (one per
        // process, scattered in memory) only when the value would not change.
        if s.skip_ok && s.cur_code(r.offset as usize) == code && {
            let ds = &s.drivers[info.slot as usize];
            ds.value.code((r.offset - ds.range.offset) as usize) == code
                && (!ds.shared || s.sole(r))
        } {
            w.phantom.push(r);
            self.st.act |= ACT_ASSIGNED;
            self.st.phantoms.push(info.local);
            return true;
        }
        let now = self.sh.now;
        w.quick = Some(Quick {
            time: now,
            elem: r.offset,
            code,
        });
        self.st.act |= ACT_ASSIGNED;
        self.st.scheduled.push((now, d));
        true
    }

    /// `assign_code` of `code` to the one element `r` of driver `d` with zero delay (transport
    /// or inertial without rejection): true when the usual fast case took it, false (nothing
    /// done) where `assign_code` must run, which also produces every error.
    #[inline]
    pub fn assign_code_now(&mut self, d: DriverId, r: Range, code: u8) -> bool {
        self.assign_code_fast(d, Some(r), Time::ZERO, code, DelayMode::Transport)
    }

    /// `assign_code_now` where the assignment would change nothing (the driver and the signal
    /// have `code`, the signal allows skipping, nothing is pending): it is skipped as
    /// `assign_code_fast` skips it. False, nothing done, otherwise: `assign_code_now` must run.
    #[inline]
    pub fn skip_code_now(&mut self, d: DriverId, r: Range, code: u8) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        if code >= 9 || info.owner != self.pid || r.len != 1 || !info.range.contains(r) {
            return false;
        }
        let w = &self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        let s = &self.sh.signals[info.signal.0 as usize];
        if info.direct {
            let same = !self.has_entries(info, 1)
                && !self.has_entries(info, 2)
                && s.direct_same(info.slot, r.offset, code);
            if same {
                self.note_skipped();
            }
            return same;
        }
        let same = matches!(s.ty.elem, crate::value::Elem::Logic)
            && s.skip_ok
            && s.cur_code(r.offset as usize) == code
            && {
                let ds = &s.drivers[info.slot as usize];
                ds.value.code((r.offset - ds.range.offset) as usize) == code
                    && (!ds.shared || s.sole(r))
            };
        if same {
            self.st.waveforms[info.local as usize].phantom.push(r);
            self.st.act |= ACT_ASSIGNED;
            self.st.phantoms.push(info.local);
        }
        same
    }

    /// An assignment to a direct driver was skipped: the run's delta stays (the kernel's
    /// `empty_delta`), the driver's waveform does not change, so later assignments of the run
    /// still take the fast path (a skipped assignment of a direct driver leaves nothing to
    /// preempt: the entries of the run are the last assignment that wins).
    #[inline]
    fn note_skipped(&mut self) {
        if self.st.phantoms.last() != Some(&SKIPPED) {
            self.st.act |= ACT_ASSIGNED;
            self.st.phantoms.push(SKIPPED);
        }
    }

    /// `assign_code_now` for `n` elements, `run(k)` being the offset in the signal and the code of
    /// the `k`th, in order, with the driver and its waveform looked up once: how many of them,
    /// from the front, the usual fast case took (the caller runs `assign_code` for the next one).
    /// A driver that is not direct takes one element at most: the first leaves a quick transaction.
    pub fn assign_codes_now(
        &mut self,
        d: DriverId,
        n: usize,
        run: impl Fn(usize) -> (u32, u8),
    ) -> usize {
        if n == 0 {
            return 0;
        }
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return 0;
        };
        if !info.direct {
            let (elem, code) = run(0);
            return usize::from(self.assign_code_now(d, Range::new(elem, 1), code));
        }
        let w = &self.st.waveforms[info.local as usize];
        if info.owner != self.pid || !w.idle() || !w.phantom.is_empty() || self.has_entries(info, 2)
        {
            return 0;
        }
        let (sig, slot, range) = (info.signal.0, info.slot, info.range);
        // As `assign_code_fast`: an element the signal and the driver hold already makes no entry
        // while nothing of this run is pending for it (no entry of the driver before this call,
        // none for the element in it; the batch holds at most `DRIVE_BATCH` elements).
        let fresh = !self.has_entries(info, 1);
        let first = self.st.out.len();
        let mut taken = 0;
        while taken < n {
            let (elem, code) = run(taken);
            if code >= 9 || !range.contains(Range::new(elem, 1)) {
                break;
            }
            taken += 1;
            if fresh
                && self.sh.signals[sig as usize].direct_same(slot, elem, code)
                && !self.st.out[first..].iter().any(|o| o.elem == elem)
            {
                self.note_skipped();
                continue;
            }
            self.st.act |= ACT_ASSIGNED;
            self.st.out.push(Out {
                sig,
                slot,
                elem,
                code: u32::from(code),
            });
        }
        if self.st.out.len() > first {
            self.mark_entries(info, 1);
        }
        taken
    }

    /// `assign_bytes` of one element with zero delay (transport or inertial without rejection):
    /// true when the usual fast case took it, false (nothing done) where `assign_bytes` must
    /// run, which also produces every error.
    #[inline]
    pub fn assign_bytes_now(&mut self, d: DriverId, r: Range, bytes: &[u8]) -> bool {
        self.assign_bytes_fast(d, Some(r), Time::ZERO, bytes, DelayMode::Transport)
    }

    /// `assign_blocks` of at most 64 elements given as their four planes to `r` (the whole driver
    /// when `None`) of driver `d` with
    /// zero delay (transport or inertial without rejection): true when the usual fast case took
    /// it, false (nothing done) where `assign_blocks` must run, which also produces every error.
    #[inline]
    pub fn assign_planes_now(&mut self, d: DriverId, r: Option<Range>, planes: [u64; 4]) -> bool {
        self.assign_planes_fast(d, r, Time::ZERO, planes, None, DelayMode::Transport)
    }

    /// `assign` of a logic value of at most 64 elements given as the plane blocks `read_into`
    /// produces for it: four little-endian `u64` planes (`blocks` is exactly 32 bytes for up to
    /// 64 elements, 32 per 64 more), unused bits zero. No `LogicVec` or `Value` is needed.
    #[inline]
    pub fn assign_blocks(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        blocks: &[u8],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        // One word of 64 elements or less: the usual vector.
        if let [b] = blocks.as_chunks::<32>().0
            && blocks.len() == 32
            && self.assign_planes_fast(d, target, delay, words(b), None, mode)
        {
            return Ok(());
        }
        self.assign_blocks_slow(d, target, delay, blocks, mode)
    }

    #[inline(never)]
    fn assign_blocks_slow(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        blocks: &[u8],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        if blocks.len() > 32 && self.assign_wide_fast(d, target, delay, blocks, mode) {
            return Ok(());
        }
        let (_, r) = self.target(d, target)?;
        let n = r.len as usize;
        if blocks.len() != n.div_ceil(64) * 32 {
            return Err(self.error(ErrorKind::InvalidAssign(format!(
                "{} plane bytes for {n} elements",
                blocks.len()
            ))));
        }
        let blocks: Vec<[u64; 4]> = blocks.as_chunks::<32>().0.iter().map(words).collect();
        let v = LogicVec::from_repr(yavs_logic::Planes::from_blocks(n, blocks));
        self.assign(d, target, &[(delay, &v)], mode)
    }

    /// `assign` of one element of a byte signal (an enum, an integer, a boolean, a real: at
    /// most 8 bytes) with one waveform element, given as its bytes: the usual scalar assignment
    /// without building a value or a waveform list.
    #[inline]
    pub fn assign_bytes(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        bytes: &[u8],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        if self.assign_bytes_fast(d, target, delay, bytes, mode) {
            return Ok(());
        }
        self.assign_bytes_slow(d, target, delay, bytes, mode)
    }

    #[inline(never)]
    fn assign_bytes_slow(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        bytes: &[u8],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        self.assign(d, target, &[(delay, bytes)], mode)
    }

    /// The common scalar byte case of `assign` in one pass: one element to a driver with nothing
    /// pending is kept as a `BQuick` (an unchanged zero-delay one is skipped as in `assign`).
    /// False when the general path must run (it also produces every error).
    #[inline(always)]
    fn assign_bytes_fast(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        bytes: &[u8],
        mode: DelayMode,
    ) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        let r = target.unwrap_or(info.range);
        let at = self.fast_time(delay, mode);
        let n = bytes.len();
        if at.is_none()
            || info.owner != self.pid
            || r.len != 1
            || n == 0
            || n > 8
            || !info.range.contains(r)
        {
            return false;
        }
        let sig = info.signal.0 as usize;
        if !matches!(self.sh.signals[sig].ty.elem, crate::value::Elem::Bytes(es) if es as usize == n)
        {
            return false;
        }
        let local = info.local as usize;
        let w = &mut self.st.waveforms[local];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        let now = self.sh.now;
        let at = at.unwrap_or(now);
        if info.bdirect {
            if at == now {
                // The commit writes it to the signal; a later assignment of the run to the driver
                // is another entry (the last one wins), one that needs the waveform moves them.
                self.mark_entries(info, 3);
                self.st.act |= ACT_ASSIGNED;
                self.st.outb.push(OutB {
                    sig: sig as u32,
                    slot: info.slot,
                    elem: r.offset,
                    es: n as u32,
                    val: pack8(bytes),
                });
                return true;
            }
            // The entries commit at the end of the run, before this one: the general path
            // orders them (inertial preemption must see them).
            if self.direct_entries(info) {
                return false;
            }
        }
        if at == now && self.unchanged(sig, d, r, ValueRef::Bytes(bytes)) {
            let w = &mut self.st.waveforms[local];
            w.phantom.push(r);
            self.st.act |= ACT_ASSIGNED;
            self.st.phantoms.push(local as u32);
            return true;
        }
        let val = pack8(bytes);
        self.st.waveforms[local].bquick = Some(BQuick {
            time: at,
            elem: r.offset,
            es: n as u8,
            val,
        });
        self.st.act |= ACT_ASSIGNED;
        self.st.scheduled.push((at, d));
        true
    }

    /// The common vector case of `assign` in one pass: a zero-delay assignment of at most 64
    /// logic elements (`planes`, `len` elements) to a driver with nothing pending is kept as a
    /// `VQuick`, no list entry and no value buffer. False when the general path must run (it
    /// also produces every error).
    #[inline(always)]
    fn assign_planes_fast(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        planes: [u64; 4],
        want: Option<u32>,
        mode: DelayMode,
    ) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        let r = target.unwrap_or(info.range);
        let at = self.fast_time(delay, mode);
        let len = r.len;
        if at.is_none()
            || info.owner != self.pid
            || want.is_some_and(|w| w != len)
            || len == 0
            || len > 64
            || !info.range.contains(r)
        {
            return false;
        }
        let w = &mut self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        let now = self.sh.now;
        let at = at.unwrap_or(now);
        if at > now {
            if (info.direct && self.direct_entries(info))
                || !matches!(
                    self.sh.signals[info.signal.0 as usize].ty.elem,
                    crate::value::Elem::Logic
                )
            {
                return false;
            }
            let planes = planes.map(|p| p & yavs_logic::wide::nmask(len as usize));
            self.st.waveforms[info.local as usize].vquick = Some(VQuick {
                time: at,
                elem: r.offset,
                len,
                planes,
            });
            self.st.act |= ACT_ASSIGNED;
            self.st.scheduled.push((at, d));
            return true;
        }
        if info.direct {
            let (sig, slot) = (info.signal.0, info.slot);
            let planes = planes.map(|p| p & yavs_logic::wide::nmask(len as usize));
            // One-element entries of the driver (the fields of a record before this one) are
            // applied before the vector entries, which is the order they were made in.
            // Nothing of this run on the driver and the signal and its driving value have the
            // value: the commit would only set the elements pending (`direct_same_planes`).
            if !self.has_entries(info, 1)
                && !self.has_entries(info, 2)
                && self.sh.signals[sig as usize].direct_same_planes(slot, r.offset, len, planes)
            {
                self.note_skipped();
                return true;
            }
            self.mark_entries(info, 2);
            self.st.act |= ACT_ASSIGNED;
            self.st.outv.push(OutV {
                sig,
                slot,
                elem: r.offset,
                len,
                planes,
            });
            return true;
        }
        let s = &self.sh.signals[info.signal.0 as usize];
        let ds = &s.drivers[info.slot as usize];
        let (Buf::Planes { b: dv, .. }, Buf::Planes { b: cv, .. }) = (&ds.value, &s.cur) else {
            return false;
        };
        let planes = planes.map(|p| p & yavs_logic::wide::nmask(len as usize));
        let rel = (r.offset - ds.range.offset) as usize;
        let n = len as usize;
        if s.skip_ok
            && yavs_logic::wide::get_block(dv, rel, n) == planes
            && yavs_logic::wide::get_block(cv, r.offset as usize, n) == planes
            && (!ds.shared || s.sole(r))
        {
            w.phantom.push(r);
            self.st.act |= ACT_ASSIGNED;
            self.st.phantoms.push(info.local);
            return true;
        }
        let now = self.sh.now;
        w.vquick = Some(VQuick {
            time: now,
            elem: r.offset,
            len,
            planes,
        });
        self.st.act |= ACT_ASSIGNED;
        self.st.scheduled.push((now, d));
        true
    }

    /// A zero-delay assignment of more than 64 logic elements (`blocks`, as in `assign_blocks`) to
    /// a direct driver with nothing pending: one `OutV` entry per plane word, committed at the end
    /// of the run like those of a short vector (a wide register or bus). False (nothing done)
    /// where the general path must run (it also produces every error).
    #[inline(never)]
    fn assign_wide_fast(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        delay: Time,
        blocks: &[u8],
        mode: DelayMode,
    ) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        let r = target.unwrap_or(info.range);
        let n = r.len as usize;
        if !info.direct
            || info.owner != self.pid
            || n <= 64
            || blocks.len() != n.div_ceil(64) * 32
            || !info.range.contains(r)
            || self.fast_time(delay, mode) != Some(self.sh.now)
        {
            return false;
        }
        let w = &self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        self.mark_entries(info, 2);
        let (sig, slot) = (info.signal.0, info.slot);
        for (i, b) in blocks.as_chunks::<32>().0.iter().enumerate() {
            let len = (n - 64 * i).min(64);
            self.st.act |= ACT_ASSIGNED;
            self.st.outv.push(OutV {
                sig,
                slot,
                elem: r.offset + 64 * i as u32,
                len: len as u32,
                planes: words(b).map(|p| p & yavs_logic::wide::nmask(len)),
            });
        }
        true
    }

    /// The time a fast-path assignment takes effect: `now` for a zero delay (transport, or
    /// inertial without rejection), a later time for a positive delay with the same modes;
    /// `None` where the general path must run (it also produces every error).
    #[inline(always)]
    fn fast_time(&self, delay: Time, mode: DelayMode) -> Option<Time> {
        match mode {
            DelayMode::Inertial { reject: Some(t) } if t != Time::ZERO => return None,
            _ => {}
        }
        if delay == Time::ZERO {
            return Some(self.sh.now);
        }
        self.after(delay).ok().filter(|&t| t > self.sh.now)
    }

    /// A transaction of one logic element of the process's own idle driver at `at`, later than
    /// now, when the run has no direct entry yet: a quick transaction, as `assign_code_fast`
    /// makes it. False (nothing done) otherwise; the caller then uses `assign_code`.
    #[inline]
    fn assign_later(&mut self, d: DriverId, elem: u32, at: Time, code: u8) -> bool {
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return false;
        };
        if info.owner != self.pid
            || code >= 9
            || !self.st.out.is_empty()
            || !self.st.outv.is_empty()
            || !self.st.outb.is_empty()
            || !info.range.contains(Range::new(elem, 1))
            || !matches!(
                self.sh.signals[info.signal.0 as usize].ty.elem,
                crate::value::Elem::Logic
            )
        {
            return false;
        }
        let w = &mut self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        w.quick = Some(Quick {
            time: at,
            elem,
            code,
        });
        self.st.act |= ACT_ASSIGNED;
        self.st.scheduled.push((at, d));
        true
    }

    /// The process has a direct entry for `slot` of signal `sig` in this run (they commit at its
    /// end, so a delayed assignment must come after them: the general path orders it).
    #[inline(always)]
    fn direct_entries(&self, info: &DriverInfo) -> bool {
        let Some(m) = &self.st.marks else {
            let key = (info.signal.0, info.slot);
            return self.st.out.iter().any(|o| (o.sig, o.slot) == key)
                || self.st.outv.iter().any(|o| (o.sig, o.slot) == key)
                || self.st.outb.iter().any(|o| (o.sig, o.slot) == key);
        };
        m.d.get(info.local as usize)
            .is_some_and(|&x| x >> 2 == m.epoch && x & 3 != 0)
    }

    /// Driver `local` has `kind` (1 `out`, 2 `outv`, 3 `outb`) entries in this run.
    #[inline(always)]
    fn has_entries(&self, info: &DriverInfo, kind: u64) -> bool {
        let Some(m) = &self.st.marks else {
            let key = (info.signal.0, info.slot);
            return match kind {
                1 => self.st.out.iter().any(|o| (o.sig, o.slot) == key),
                2 => self.st.outv.iter().any(|o| (o.sig, o.slot) == key),
                _ => self.st.outb.iter().any(|o| (o.sig, o.slot) == key),
            };
        };
        m.d.get(info.local as usize) == Some(&(m.epoch << 2 | kind))
    }

    /// Driver `local` has `kind` entries now.
    #[inline(always)]
    fn mark_entries(&mut self, info: &DriverInfo, kind: u64) {
        if let Some(m) = &mut self.st.marks {
            let v = m.epoch << 2 | kind;
            if let Some(x) = m.d.get_mut(info.local as usize) {
                *x = v;
            }
        }
    }

    /// Whether a zero-delay assignment of `v` to `r` of driver `d` would change nothing: the
    /// signal allows skipping (`Config::skip_unchanged`), `d` has nothing pending, `r` has no
    /// other source, and the driver and the signal already hold `v`.
    fn unchanged(&self, sig: usize, d: DriverId, r: Range, v: ValueRef<'_>) -> bool {
        let s = &self.sh.signals[sig];
        let Ok(info) = self.own(d) else {
            return false;
        };
        if !s.skip_ok || !self.st.waveforms[info.local as usize].idle() || r.len == 0 {
            return false;
        }
        let ds = &s.drivers[info.slot as usize];
        // The value of a lagging driver is in the driving value (`Signal::lag`).
        if s.lag && !ds.shared {
            return false;
        }
        let (rel, n) = ((r.offset - ds.range.offset) as usize, r.len as usize);
        // The cheap tests first: most assignments of a running design change something.
        ds.value.same_as(rel, n, v)
            && s.cur.same_as(r.offset as usize, n, v)
            && (!ds.shared || s.sole(r))
    }

    /// The skipped assignments of driver `d` this run become real transactions again, before
    /// a later assignment of the run preempts them as it would have.
    fn flush_phantoms(&mut self, d: DriverId) -> Result<(), SimError> {
        let info = self.own(d)?;
        let (local, sig, slot) = (
            info.local as usize,
            info.signal.0 as usize,
            info.slot as usize,
        );
        let ps = std::mem::take(&mut self.st.waveforms[local].phantom);
        let ds = &self.sh.signals[sig].drivers[slot];
        for r in ps {
            let rel = (r.offset - ds.range.offset) as usize;
            let value = Some(ds.value.extract(rel, r.len as usize));
            let time = self.sh.now;
            let mut t = TxnList::new();
            t.push(Txn {
                time,
                range: r,
                value,
            });
            self.st.waveforms[local].schedule(r, Some(time), &mut t);
            self.st.act |= ACT_ASSIGNED;
            self.st.scheduled.push((time, d));
        }
        Ok(())
    }

    /// `d <= value;` on the whole driver (zero-delay inertial).
    pub fn drive<'v>(
        &mut self,
        d: DriverId,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        let v = value.into();
        self.assign(d, None, &[(Time::ZERO, v)], DelayMode::INERTIAL)
    }

    /// A null transaction (`s <= null after T`) on a guarded signal's driver: the elements
    /// disconnect after `after`, or after the signal's disconnection time when `None`.
    pub fn disconnect(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        after: Option<Time>,
        mode: DelayMode,
    ) -> Result<(), SimError> {
        self.spill_out(d);
        let (sig, r) = self.target(d, target)?;
        let s = &self.sh.signals[sig];
        if s.guard.is_none() {
            return Err(self.error(ErrorKind::InvalidAssign(format!(
                "null transaction on unguarded signal {}",
                s.name
            ))));
        }
        let delay = after.unwrap_or(s.disconnect);
        let mut new = TxnList::new();
        self.push_txn(&mut new, r, delay, None)?;
        self.schedule(d, r, &mut new, mode)
    }

    fn target(&self, d: DriverId, target: Option<Range>) -> Result<(usize, Range), SimError> {
        let info = self.own(d)?;
        let r = target.unwrap_or(info.range);
        if r.offset.checked_add(r.len).is_none() || !info.range.contains(r) {
            return Err(self.error(ErrorKind::InvalidAssign("target outside the driver".into())));
        }
        Ok((info.signal.0 as usize, r))
    }

    /// Append one waveform element, checking ascending times.
    fn push_txn(
        &self,
        new: &mut TxnList,
        r: Range,
        delay: Time,
        value: Option<Buf>,
    ) -> Result<(), SimError> {
        // A transaction past TIME'HIGH can never mature: dropped, as most reference
        // simulators do (lrm-checks.md).
        let Ok(time) = self.after(delay) else {
            return Ok(());
        };
        if new.last().is_some_and(|t| t.time >= time) {
            return Err(self.error(ErrorKind::InvalidAssign(
                "waveform delays not ascending".into(),
            )));
        }
        new.push(Txn {
            time,
            range: r,
            value,
        });
        Ok(())
    }

    fn schedule(
        &mut self,
        d: DriverId,
        r: Range,
        new: &mut TxnList,
        mode: DelayMode,
    ) -> Result<(), SimError> {
        self.flush_phantoms(d)?;
        let fail = |m: &str| self.error(ErrorKind::InvalidAssign(m.into()));
        let Some(first) = new.first() else {
            return Ok(()); // every element was past TIME'HIGH (push_txn)
        };
        let reject_from = match mode {
            DelayMode::Transport => None,
            DelayMode::Inertial { reject } => {
                let first_delay = first.time.since(self.sh.now);
                let reject = reject.map_or(first_delay, |r| r.quantize(self.sh.resolution));
                if reject > first_delay {
                    return Err(fail("pulse rejection limit exceeds the delay"));
                }
                Some(first.time.since(reject))
            }
        };
        if r.len == 0 {
            return Ok(()); // a null slice: nothing to drive
        }
        let local = self.own(d)?.local as usize;
        self.st.act |= ACT_ASSIGNED;
        self.st.scheduled.extend(new.iter().map(|t| (t.time, d)));
        self.st.waveforms[local].schedule(r, reject_from, new);
        Ok(())
    }

    /// Add `r` to the sensitivity of the coming wait.
    #[inline(always)]
    pub fn wait_on(&mut self, r: impl Into<SigRef>) -> Result<(), SimError> {
        let r = r.into();
        let range = self.sig(r)?.1;
        // What was named again before is in `sens` now, ahead of this range.
        if self.st.matched != 0 {
            let st = &mut *self.st;
            st.sens
                .extend_from_slice(&st.sens_prev[..st.matched as usize]);
            st.matched = 0;
            st.again = false;
        }
        // Most waits name one signal: no room for four (the list becomes the previous one).
        if self.st.sens.capacity() == 0 {
            self.st.sens.reserve_exact(1);
        }
        self.st.sens.push((r.signal, range));
        Ok(())
    }

    /// The coming wait resumes this process only in a cycle where the edge `kind` of the one
    /// element `r` refers to holds (`Inst::WaitEdge`): the kernel leaves it suspended for
    /// events of the other signals of the wait in other cycles. Ignored for a process whose
    /// edge trigger the elaborator made (it has one already) and for a range of more or fewer
    /// than one element.
    #[inline(always)]
    pub fn wait_edge(&mut self, r: impl Into<SigRef>, kind: u8) {
        let r = r.into();
        if self.st.known_edge.is_none()
            && let Some(x) = r.range
            && x.len == 1
            && x.offset < 1 << 30
        {
            self.st.wait_key = edge_key(r.signal, x.offset, kind);
        }
    }

    /// `wait_on` of one range, when it is the next of the ranges the last wait named, in the
    /// same order, with no timeout, and nothing was added to this one: the commit keeps the
    /// registered waiters and `sens` stays empty (a wait that ends after a part of them takes
    /// that part from the last one). `false` (nothing done) otherwise; the caller then uses
    /// `wait_on`.
    #[inline(always)]
    pub fn wait_on_again(&mut self, signal: SignalId, range: Range) -> bool {
        let st = &mut *self.st;
        if st.sens.is_empty()
            && st.deadline.is_none()
            && let Some(next) = st.sens_prev.get(st.matched as usize)
            && *next == (signal, range)
        {
            st.matched += 1;
            st.again = st.matched as usize == st.sens_prev.len();
            return true;
        }
        false
    }

    /// The same wait as the last one (its sensitivity, no timeout), when that one had no
    /// timeout and nothing was added to this one: the commit keeps the registered waiters, as
    /// `wait_on_again` does. `false` (nothing done) otherwise; the caller then names the wait
    /// with `wait_on`.
    #[inline(always)]
    pub fn wait_same(&mut self) -> bool {
        let st = &mut *self.st;
        if st.sens.is_empty() && st.deadline.is_none() && !st.sens_prev.is_empty() {
            st.again = true;
            st.matched = st.sens_prev.len() as u32;
            return true;
        }
        false
    }

    /// Timeout of the coming wait, relative to now.
    /// A timeout past TIME'HIGH never expires (lrm-checks.md).
    #[inline]
    pub fn wait_for(&mut self, delay: Time) -> Result<(), SimError> {
        self.st.act |= ACT_SLOW;
        self.st.deadline = self.after(delay).ok();
        Ok(())
    }

    /// Timeout of the coming wait as an absolute time, to re-suspend a `wait until` with its
    /// original deadline.
    #[inline]
    pub fn wait_at(&mut self, deadline: Time) -> Result<(), SimError> {
        if deadline < self.sh.now {
            return Err(self.error(ErrorKind::DeadlineInPast { deadline }));
        }
        self.st.act |= ACT_SLOW;
        self.st.deadline = Some(deadline);
        Ok(())
    }

    /// The process resumed because its timeout expired.
    #[inline]
    pub fn timed_out(&self) -> bool {
        self.st.timed_out
    }

    /// `report` / `assert`.
    pub fn report(&mut self, severity: Severity, msg: impl Into<String>) {
        let msg = msg.into();
        let live = match self.sh.live {
            Some(l) if self.live => {
                let name = &self.sh.names[self.pid.0 as usize];
                l.report(self.sh.now, self.sh.delta, name, severity, &msg);
                true
            }
            _ => false,
        };
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Report(severity, msg, live));
    }

    /// Buffered textio `OUTPUT` text, delivered to observers at commit in elaboration order
    /// (ordered with `report`). Disk files are not kernel state: their users are serial.
    pub fn output(&mut self, bytes: &[u8]) {
        let live = match self.sh.live {
            Some(l) if self.live => {
                l.output(bytes);
                true
            }
            _ => false,
        };
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Output(bytes.to_vec(), live));
    }

    /// `std.env.stop`.
    pub fn stop(&mut self, status: i32) {
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Stop(status));
    }

    /// `std.env.finish`.
    pub fn finish(&mut self, status: i32) {
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Finish(status));
    }

    /// `s <= force [in|out] value;`
    pub fn force<'v>(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        let op = make_op(
            self.sh.signals,
            r.into(),
            OpMode::Force(mode),
            Some(value.into()),
        )
        .map_err(|k| self.error(k))?;
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }

    /// A deposit (VHPI `vhpiDepositPropagate` from a foreign subprogram the process calls):
    /// the driving value of `r` until a source next updates it, applied in the next cycle.
    pub fn deposit<'v>(
        &mut self,
        r: impl Into<SigRef>,
        value: impl Into<ValueRef<'v>>,
    ) -> Result<(), SimError> {
        let op = make_op(
            self.sh.signals,
            r.into(),
            OpMode::Deposit,
            Some(value.into()),
        )
        .map_err(|k| self.error(k))?;
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }

    /// `s <= release [in|out];`
    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        let op = make_op(self.sh.signals, r.into(), OpMode::Release(mode), None)
            .map_err(|k| self.error(k))?;
        self.st.act |= ACT_SLOW;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }
}

/// Up to 8 bytes padded with zeros, built in registers for the usual sizes (a variable-length
/// copy and a wide read of it straight after is a store-forwarding stall).
#[inline(always)]
fn pack8(b: &[u8]) -> [u8; 8] {
    if let Ok(x) = <[u8; 4]>::try_from(b) {
        return u64::from(u32::from_le_bytes(x)).to_le_bytes();
    }
    if let Ok(x) = <[u8; 8]>::try_from(b) {
        return x;
    }
    let mut v = [0u8; 8];
    for (d, s) in v.iter_mut().zip(b) {
        *d = *s;
    }
    v
}

/// Resolve a reference against the model: the signal must exist and the range must lie
/// within it (null ranges allowed).
#[inline(always)]
/// The four little-endian plane words of a 32-byte block.
fn words(b: &[u8; 32]) -> [u64; 4] {
    let (w, _) = b.as_chunks::<8>();
    std::array::from_fn(|k| u64::from_le_bytes(w[k]))
}

#[inline(always)]
pub(crate) fn check_ref(signals: &[Signal], r: SigRef) -> Result<(&Signal, Range), ErrorKind> {
    if let Some(s) = signals.get(r.signal.0 as usize) {
        let range = s.resolve(r.range);
        if range.offset.checked_add(range.len).is_some() && s.full().contains(range) {
            return Ok((s, range));
        }
    }
    Err(bad_ref(signals, r))
}

/// The error of a reference `check_ref` refused.
#[cold]
fn bad_ref(signals: &[Signal], r: SigRef) -> ErrorKind {
    let Some(s) = signals.get(r.signal.0 as usize) else {
        return ErrorKind::BadRef(format!("no signal {}", r.signal.0));
    };
    let range = s.resolve(r.range);
    ErrorKind::BadRef(format!(
        "elements {}+{} out of range for {} (length {})",
        range.offset, range.len, s.name, s.ty.len
    ))
}

/// Validate a force/release/deposit request.
pub(crate) fn make_op(
    signals: &[Signal],
    r: SigRef,
    mode: OpMode,
    value: Option<ValueRef<'_>>,
) -> Result<SignalOp, ErrorKind> {
    let (s, range) = check_ref(signals, r)?;
    let by_port = |m: ForceMode| match m {
        ForceMode::Default if s.out_port => ForceMode::Driving,
        ForceMode::Default => ForceMode::Effective,
        m => m,
    };
    let mode = match mode {
        OpMode::Force(m) => OpMode::Force(by_port(m)),
        OpMode::Release(m) => OpMode::Release(by_port(m)),
        m => m,
    };
    let value = match (mode, value) {
        (OpMode::Release(_), None) => None,
        (OpMode::Force(_) | OpMode::Deposit, Some(v)) => Some(
            Buf::from_ref(s.ty.elem, range.len as usize, v)
                .map_err(|m| ErrorKind::BadRef(format!("{}: {m}", s.name)))?,
        ),
        _ => return Err(ErrorKind::Internal("force/deposit without a value".into())),
    };
    Ok(SignalOp {
        signal: r.signal,
        range,
        mode,
        value,
    })
}
