//! The process interface the interpreter and the LLVM tier implement.

use crate::error::{ErrorKind, SimError};
use crate::signal::{
    DelayMode, DriverId, ForceMode, Quick, Range, SigRef, Signal, SignalId, Txn, TxnList, Waveform,
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

/// Per-process state the process mutates while it runs.
#[derive(Default)]
pub(crate) struct ProcState {
    pub waveforms: SmallVec<[Waveform; 1]>,
    pub effects: Vec<Effect>,
    pub sens: SmallVec<[(SignalId, Range); 2]>,
    /// The sensitivity the last commit registered, when that wait had no timeout.
    pub sens_prev: SmallVec<[(SignalId, Range); 2]>,
    pub deadline: Option<Time>,
    pub timed_out: bool,
    /// Times of transactions scheduled during this run.
    pub scheduled: SmallVec<[(Time, DriverId); 2]>,
    /// Drivers (locals) that hold skipped unchanged assignments (`Waveform::phantom`) from
    /// this run.
    pub phantoms: Vec<u32>,
    /// The edge the kernel resumed the process for (`Builder::edge_trigger`: signal, element,
    /// rising): true for the whole activation, so `Ctx::edge` of it need not look.
    pub known_edge: Option<(SignalId, u32, bool)>,
    /// A one-element logic value kept for `Ctx::assign_code`.
    pub one: Option<LogicVec>,
}

impl ProcState {
    /// End of a run: whether skipped assignments are left, which the kernel must replace by an
    /// empty delta.
    pub fn take_phantoms(&mut self) -> bool {
        let mut any = false;
        for l in self.phantoms.drain(..) {
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
    pub fn now(&self) -> Time {
        self.sh.now
    }

    pub fn delta(&self) -> u32 {
        self.sh.delta
    }

    /// `Config::stop_severity`: a report at this severity ends the simulation after the
    /// current cycle, so the process should not execute past it.
    pub fn stop_severity(&self) -> Severity {
        self.sh.stop_severity
    }

    pub fn process(&self) -> ProcId {
        self.pid
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
    #[inline]
    pub fn read_into(&self, r: impl Into<SigRef>, out: &mut [u8]) -> Result<(), SimError> {
        let (s, r) = self.sig(r)?;
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
        Ok(s.last_event(r, self.sh.now))
    }

    /// `'last_active`.
    pub fn last_active(&self, r: impl Into<SigRef>) -> Result<Time, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.last_active(r, self.sh.now))
    }

    /// `'last_value` of the range as a whole.
    pub fn last_value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.last_value_of(r).to_value())
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

    /// The bytes of the `std_ulogic` codes of the current value of logic signal `s`, one per
    /// element, as a pointer and the number of elements, for compiled code that reads them
    /// itself. `None` for a signal that keeps no codes (bytes, or over `CODES_MAX` elements).
    /// The pointer stays valid for the whole simulation; the bytes change only in the update
    /// phase, while no process runs.
    pub fn code_view(&self, s: SignalId) -> Option<(*const u8, usize)> {
        let sig = self.sh.signals.get(s.0 as usize)?;
        sig.codes_view()
    }

    /// The edge the kernel resumed this process for in this activation (signal, element,
    /// rising), if `Ctx::edge` takes it as held.
    pub fn known_edge(&self) -> Option<(SignalId, u32, bool)> {
        self.st.known_edge
    }

    /// `rising_edge` / `falling_edge` of the first element of a logic signal range: an event
    /// in this cycle, `To_X01` of the current value the new level and `To_X01` of the
    /// `'last_value` the old one; `None` for a null range. One reference check instead of
    /// the three of `event` and two `first_code`.
    #[inline(always)]
    pub fn edge(&self, r: impl Into<SigRef>, rising: bool) -> Result<Option<bool>, SimError> {
        let r = r.into();
        if let (Some((sig, el, up)), Some(x)) = (self.st.known_edge, r.range)
            && r.signal == sig
            && x.offset == el
            && x.len == 1
            && up == rising
        {
            return Ok(Some(true));
        }
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

    /// `'driving`: false if any element of this process's driver is disconnected.
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
        Ok(
            self.sh.signals[info.signal.0 as usize].drivers[info.slot as usize]
                .value
                .to_value(),
        )
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
    #[inline]
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
        let now_mode = delay == Time::ZERO
            && match mode {
                DelayMode::Transport => true,
                DelayMode::Inertial { reject } => reject.is_none_or(|t| t == Time::ZERO),
            };
        if !now_mode || code >= 9 || info.owner != self.pid || r.len != 1 || !info.range.contains(r)
        {
            return false;
        }
        let w = &mut self.st.waveforms[info.local as usize];
        if !w.idle() || !w.phantom.is_empty() {
            return false;
        }
        let s = &self.sh.signals[info.signal.0 as usize];
        if !matches!(s.ty.elem, crate::value::Elem::Logic) {
            return false;
        }
        let ds = &s.drivers[info.slot as usize];
        let rel = (r.offset - ds.range.offset) as usize;
        if s.skip_ok
            && ds.value.code(rel) == code
            && s.cur_code(r.offset as usize) == code
            && (!ds.shared || s.sole(r))
        {
            w.phantom.push(r);
            self.st.phantoms.push(info.local);
            return true;
        }
        let now = self.sh.now;
        w.quick = Some(Quick {
            time: now,
            elem: r.offset,
            code,
        });
        self.st.scheduled.push((now, d));
        true
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
        self.st.scheduled.extend(new.iter().map(|t| (t.time, d)));
        self.st.waveforms[local].schedule(r, reject_from, new);
        Ok(())
    }

    /// Add `r` to the sensitivity of the coming wait.
    #[inline(always)]
    pub fn wait_on(&mut self, r: impl Into<SigRef>) -> Result<(), SimError> {
        let r = r.into();
        let range = self.sig(r)?.1;
        self.st.sens.push((r.signal, range));
        Ok(())
    }

    /// Timeout of the coming wait, relative to now.
    /// A timeout past TIME'HIGH never expires (lrm-checks.md).
    pub fn wait_for(&mut self, delay: Time) -> Result<(), SimError> {
        self.st.deadline = self.after(delay).ok();
        Ok(())
    }

    /// Timeout of the coming wait as an absolute time, to re-suspend a `wait until` with its
    /// original deadline.
    pub fn wait_at(&mut self, deadline: Time) -> Result<(), SimError> {
        if deadline < self.sh.now {
            return Err(self.error(ErrorKind::DeadlineInPast { deadline }));
        }
        self.st.deadline = Some(deadline);
        Ok(())
    }

    /// The process resumed because its timeout expired.
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
        self.st.effects.push(Effect::Output(bytes.to_vec(), live));
    }

    /// `std.env.stop`.
    pub fn stop(&mut self, status: i32) {
        self.st.effects.push(Effect::Stop(status));
    }

    /// `std.env.finish`.
    pub fn finish(&mut self, status: i32) {
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
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }

    /// `s <= release [in|out];`
    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        let op = make_op(self.sh.signals, r.into(), OpMode::Release(mode), None)
            .map_err(|k| self.error(k))?;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }
}

/// Resolve a reference against the model: the signal must exist and the range must lie
/// within it (null ranges allowed).
#[inline(always)]
pub(crate) fn check_ref(signals: &[Signal], r: SigRef) -> Result<(&Signal, Range), ErrorKind> {
    let s = signals
        .get(r.signal.0 as usize)
        .ok_or_else(|| ErrorKind::BadRef(format!("no signal {}", r.signal.0)))?;
    let range = s.resolve(r.range);
    if range.offset.checked_add(range.len).is_none() || !s.full().contains(range) {
        let msg = format!(
            "elements {}+{} out of range for {} (length {})",
            range.offset, range.len, s.name, s.ty.len
        );
        return Err(ErrorKind::BadRef(msg));
    }
    Ok((s, range))
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
