//! The process interface the interpreter and the LLVM tier implement.

use crate::error::{ErrorKind, SimError};
use crate::signal::{
    DelayMode, DriverId, ForceMode, Range, SigRef, Signal, SignalId, Txn, Waveform,
};
use crate::time::{Time, Unit};
use crate::value::{Buf, LogicVec, Value, ValueRef};

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
}

impl<F: FnMut(&mut Ctx<'_>) -> Result<(), SimError> + Send> Process for F {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        self(ctx)
    }
}

/// A buffered side effect, committed after the delta in elaboration order.
pub(crate) enum Effect {
    Report(Severity, String),
    Output(Vec<u8>),
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
    pub waveforms: Vec<Waveform>,
    pub effects: Vec<Effect>,
    pub sens: Vec<(SignalId, Range)>,
    pub deadline: Option<Time>,
    pub timed_out: bool,
    /// Times of transactions scheduled during this run.
    pub scheduled: Vec<(Time, DriverId)>,
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
}

/// What a running process sees: current-cycle signal values, its own drivers and its own
/// effect buffer.
pub struct Ctx<'a> {
    pub(crate) sh: &'a Shared<'a>,
    pub(crate) pid: ProcId,
    pub(crate) st: &'a mut ProcState,
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

    fn sig(&self, r: impl Into<SigRef>) -> Result<(&Signal, Range), SimError> {
        check_ref(self.sh.signals, r.into()).map_err(|k| self.error(k))
    }

    /// Current value of a byte signal.
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

    /// `'event`: an event on any element in this cycle.
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
        Ok(elapsed(self.sh.now, &s.last_event[r.elems()]))
    }

    /// `'last_active`.
    pub fn last_active(&self, r: impl Into<SigRef>) -> Result<Time, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(elapsed(self.sh.now, &s.last_active[r.elems()]))
    }

    /// `'last_value`, element by element.
    pub fn last_value(&self, r: impl Into<SigRef>) -> Result<Value, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.last_value
            .extract(r.offset as usize, r.len as usize)
            .to_value())
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
        let mut new = Vec::with_capacity(waveform.len());
        for &(delay, v) in waveform {
            let buf = Buf::from_ref(elem, r.len as usize, v.into())
                .map_err(|m| self.error(ErrorKind::InvalidAssign(m)))?;
            self.push_txn(&mut new, r, delay, Some(buf))?;
        }
        self.schedule(d, r, new, mode)
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
        let mut new = Vec::with_capacity(1);
        self.push_txn(&mut new, r, delay, None)?;
        self.schedule(d, r, new, mode)
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
        new: &mut Vec<Txn>,
        r: Range,
        delay: Time,
        value: Option<Buf>,
    ) -> Result<(), SimError> {
        let time = self.after(delay)?;
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
        new: Vec<Txn>,
        mode: DelayMode,
    ) -> Result<(), SimError> {
        let fail = |m: &str| self.error(ErrorKind::InvalidAssign(m.into()));
        let Some(first) = new.first() else {
            return Err(fail("empty waveform"));
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
    pub fn wait_on(&mut self, r: impl Into<SigRef>) -> Result<(), SimError> {
        let r = r.into();
        let range = self.sig(r)?.1;
        self.st.sens.push((r.signal, range));
        Ok(())
    }

    /// Timeout of the coming wait, relative to now.
    pub fn wait_for(&mut self, delay: Time) -> Result<(), SimError> {
        self.st.deadline = Some(self.after(delay)?);
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
        self.st.effects.push(Effect::Report(severity, msg.into()));
    }

    /// Buffered textio `OUTPUT` text, delivered to observers at commit in elaboration order
    /// (ordered with `report`). Disk files are not kernel state: their users are serial.
    pub fn output(&mut self, bytes: &[u8]) {
        self.st.effects.push(Effect::Output(bytes.to_vec()));
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

    /// `s <= release [in|out];`
    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        let op = make_op(self.sh.signals, r.into(), OpMode::Release(mode), None)
            .map_err(|k| self.error(k))?;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }
}

fn elapsed(now: Time, times: &[Option<Time>]) -> Time {
    times
        .iter()
        .flatten()
        .max()
        .map_or(Time::MAX, |&t| now.since(t))
}

/// Resolve a reference against the model: the signal must exist and the range must lie
/// within it (null ranges allowed).
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
