//! The process interface the interpreter and the LLVM tier implement.

use crate::error::{ErrorKind, SimError};
use crate::signal::{
    DelayMode, DriverId, ForceMode, Range, SigRef, Signal, SignalId, Txn, Waveform,
};
use crate::time::{Time, Unit};

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
/// `run` executes from the point of the last suspension until the next `wait`
/// statement, then returns. Before returning it states what it waits for with
/// [`Ctx::wait_on`] and [`Ctx::wait_for`]/[`Ctx::wait_at`]; returning with
/// neither waits forever (a terminated process or a bare `wait;`).
///
/// A `wait until` condition belongs to the process: on resumption it
/// evaluates the condition and, when false and not [`Ctx::timed_out`],
/// suspends again with the same sensitivity and the same absolute deadline.
///
/// `run` may execute on any worker thread. It must only read signals through
/// `ctx` and touch no state shared with other processes, unless the process
/// was built as `serial`.
pub trait Process: Send {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError>;
}

impl<F: FnMut(&mut Ctx<'_>) -> Result<(), SimError> + Send> Process for F {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        self(ctx)
    }
}

/// A buffered side effect, committed after the delta in elaboration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Report(Severity, String),
    Output(u32, Vec<u8>),
    Stop(i32),
    Finish(i32),
    Op(SignalOp),
}

/// A force, release or deposit, applied in the next simulation cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SignalOp {
    pub signal: SignalId,
    pub range: Range,
    pub kind: OpKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    Force(ForceMode, Vec<u8>),
    Release(ForceMode),
    Deposit(Vec<u8>),
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
    /// Index into the signal's driver list.
    pub slot: u32,
}

/// Read-only kernel state shared by all processes running in a delta.
pub(crate) struct Shared<'a> {
    pub now: Time,
    pub delta: u32,
    pub cycle: u64,
    pub resolution: Unit,
    pub signals: &'a [Signal],
    pub drivers: &'a [DriverInfo],
    pub names: &'a [String],
}

/// What a running process sees: current-cycle signal values, its own drivers
/// and its own effect buffer.
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

    pub fn process(&self) -> ProcId {
        self.pid
    }

    /// An error with this process, time and delta attached. The interpreter
    /// adds its call stack to `trace`.
    pub fn error(&self, kind: ErrorKind) -> SimError {
        SimError {
            process: Some((self.pid, self.sh.names[self.pid.0 as usize].clone())),
            ..SimError::at(kind, self.sh.now, self.sh.delta)
        }
    }

    fn sig(&self, r: impl Into<SigRef>) -> Result<(&Signal, Range), SimError> {
        check_ref(self.sh.signals, r.into()).map_err(|k| self.error(k))
    }

    /// Current (effective) value.
    pub fn value(&self, r: impl Into<SigRef>) -> Result<&[u8], SimError> {
        let (s, r) = self.sig(r)?;
        Ok(&s.cur[s.bytes(r)])
    }

    /// `'event`: an event on any element in this cycle.
    pub fn event(&self, r: impl Into<SigRef>) -> Result<bool, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.event_cycle[r.elems()].contains(&self.sh.cycle))
    }

    /// `'active`.
    pub fn active(&self, r: impl Into<SigRef>) -> Result<bool, SimError> {
        let (s, r) = self.sig(r)?;
        Ok(s.active_cycle[r.elems()].contains(&self.sh.cycle))
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
    pub fn last_value(&self, r: impl Into<SigRef>) -> Result<&[u8], SimError> {
        let (s, r) = self.sig(r)?;
        Ok(&s.last_value[s.bytes(r)])
    }

    /// `now + delay` after truncating `delay` to the resolution limit.
    pub fn after(&self, delay: Time) -> Result<Time, SimError> {
        let delay = delay.quantize(self.sh.resolution);
        let base = self.sh.now;
        base.checked_add(delay)
            .map_err(|_| self.error(ErrorKind::TimeOverflow { base, delay }))
    }

    /// Signal assignment through driver `d` to elements `target` (the whole
    /// driver when `None`). `waveform` holds `(delay, value)` pairs in
    /// ascending delay order, each value covering `target`.
    pub fn assign(
        &mut self,
        d: DriverId,
        target: Option<Range>,
        waveform: &[(Time, &[u8])],
        mode: DelayMode,
    ) -> Result<(), SimError> {
        let fail = |reason| self.error(ErrorKind::InvalidAssign(reason));
        let info = self
            .sh
            .drivers
            .get(d.0 as usize)
            .ok_or_else(|| fail("no such driver"))?;
        if info.owner != self.pid {
            return Err(fail("driver belongs to another process"));
        }
        let r = target.unwrap_or(info.range);
        if r.len == 0 || r.offset.checked_add(r.len).is_none() || !info.range.contains(r) {
            return Err(fail("target outside the driver"));
        }
        let es = self.sh.signals[info.signal.0 as usize].ty.elem_size as usize;
        let mut new = Vec::with_capacity(waveform.len());
        for &(delay, value) in waveform {
            if value.len() != r.len as usize * es {
                return Err(fail("value size does not match the target"));
            }
            let time = self.after(delay)?;
            if new.last().is_some_and(|t: &Txn| t.time >= time) {
                return Err(fail("waveform delays not ascending"));
            }
            new.push(Txn {
                time,
                range: r,
                value: value.to_vec(),
            });
        }
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
        self.st.scheduled.extend(new.iter().map(|t| (t.time, d)));
        self.st.waveforms[info.local as usize].schedule(es, r, reject_from, new);
        Ok(())
    }

    /// `d <= value;` on the whole driver (zero-delay inertial).
    pub fn drive(&mut self, d: DriverId, value: &[u8]) -> Result<(), SimError> {
        self.assign(d, None, &[(Time::ZERO, value)], DelayMode::INERTIAL)
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

    /// Timeout of the coming wait as an absolute time, to re-suspend a
    /// `wait until` with its original deadline.
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

    /// Buffered output to a stream (textio `output`, files later).
    pub fn output(&mut self, stream: u32, bytes: &[u8]) {
        self.st.effects.push(Effect::Output(stream, bytes.to_vec()));
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
    pub fn force(
        &mut self,
        r: impl Into<SigRef>,
        mode: ForceMode,
        value: &[u8],
    ) -> Result<(), SimError> {
        let op = make_op(
            self.sh.signals,
            r.into(),
            OpKind::Force(mode, value.to_vec()),
        )
        .map_err(|k| self.error(k))?;
        self.st.effects.push(Effect::Op(op));
        Ok(())
    }

    /// `s <= release [in|out];`
    pub fn release(&mut self, r: impl Into<SigRef>, mode: ForceMode) -> Result<(), SimError> {
        let op =
            make_op(self.sh.signals, r.into(), OpKind::Release(mode)).map_err(|k| self.error(k))?;
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

/// Resolve a reference against the model: the signal must exist and the
/// range must be a non-empty part of it.
pub(crate) fn check_ref(signals: &[Signal], r: SigRef) -> Result<(&Signal, Range), ErrorKind> {
    let s = signals
        .get(r.signal.0 as usize)
        .ok_or_else(|| ErrorKind::BadRef(format!("no signal {}", r.signal.0)))?;
    let range = s.resolve(r.range);
    if range.len == 0 || range.offset.checked_add(range.len).is_none() || !s.full().contains(range)
    {
        let msg = format!(
            "elements {}+{} out of range for {} (length {})",
            range.offset, range.len, s.name, s.ty.len
        );
        return Err(ErrorKind::BadRef(msg));
    }
    Ok((s, range))
}

/// Validate a force/release/deposit request.
pub(crate) fn make_op(signals: &[Signal], r: SigRef, kind: OpKind) -> Result<SignalOp, ErrorKind> {
    let (s, range) = check_ref(signals, r)?;
    if let OpKind::Force(_, v) | OpKind::Deposit(v) = &kind
        && v.len() != s.bytes(range).len()
    {
        return Err(ErrorKind::BadRef(format!(
            "value size mismatch for {}",
            s.name
        )));
    }
    Ok(SignalOp {
        signal: r.signal,
        range,
        kind,
    })
}
