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
    pub deadline: Option<Time>,
    pub timed_out: bool,
    /// Times of transactions scheduled during this run.
    pub scheduled: Vec<(Time, DriverId)>,
    /// Drivers (locals) that hold skipped unchanged assignments (`Waveform::phantom`) from
    /// this run.
    pub phantoms: Vec<u32>,
    /// The edge of the process's trigger the kernel resumes it for (`Builder::edge_trigger`:
    /// signal, element, rising; set once by `SimControl::new`), and whether this activation
    /// was such a resumption (`edge_ok`: a signal resumed it, not a timeout or the initial
    /// run): the edge then holds for the whole activation, so `Ctx::edge` of it need not look.
    pub known_edge: Option<(SignalId, u32, bool)>,
    pub edge_ok: bool,
    /// A one-element logic value kept for `Ctx::assign_code`.
    pub one: Option<LogicVec>,
    /// Direct transactions of this run: one element, and up to 64.
    pub out: Vec<Out>,
    pub outv: Vec<OutV>,
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
    /// The driver is the only source of its elements of a logic signal with no port, group,
    /// guard or resolution function (`SimControl::new`): its zero-delay quick transactions go
    /// straight to the signal at commit (`SimControl::direct`), not through the queue.
    pub direct: bool,
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
        self.st.known_edge.filter(|_| self.st.edge_ok)
    }

    /// `rising_edge` / `falling_edge` of the first element of a logic signal range: an event
    /// in this cycle, `To_X01` of the current value the new level and `To_X01` of the
    /// `'last_value` the old one; `None` for a null range. One reference check instead of
    /// the three of `event` and two `first_code`.
    #[inline(always)]
    pub fn edge(&self, r: impl Into<SigRef>, rising: bool) -> Result<Option<bool>, SimError> {
        let r = r.into();
        if let (true, Some((sig, el, up)), Some(x)) = (self.st.edge_ok, self.st.known_edge, r.range)
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

    /// Move this run's direct transactions of driver `d` (`Out`) into its waveform, in order,
    /// for an assignment that preempts or inspects pending transactions.
    pub(crate) fn spill_out(&mut self, d: DriverId) {
        if self.st.out.is_empty() && self.st.outv.is_empty() {
            return;
        }
        let Some(info) = self.sh.drivers.get(d.0 as usize) else {
            return;
        };
        let key = (info.signal.0, info.slot);
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
            self.st.scheduled.push((now, d));
        }
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
        let s = &self.sh.signals[info.signal.0 as usize];
        // A direct driver whose commits wrote only the driving value (`Signal::lag`).
        if s.lag && info.direct {
            let r = info.range;
            return Ok(s
                .driving
                .extract(r.offset as usize, r.len as usize)
                .to_value());
        }
        Ok(s.drivers[info.slot as usize].value.to_value())
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
            if (info.direct && self.direct_entries(info.signal.0, info.slot))
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
            self.st.scheduled.push((at, d));
            return true;
        }
        if info.direct {
            let (sig, slot) = (info.signal.0, info.slot);
            // The driver's vector entries keep the order: the general path moves them.
            if self.st.outv.iter().any(|o| (o.sig, o.slot) == (sig, slot)) {
                return false;
            }
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

    /// `assign_code` of `code` to the one element `r` of driver `d` with zero delay (transport
    /// or inertial without rejection): true when the usual fast case took it, false (nothing
    /// done) where `assign_code` must run, which also produces every error.
    #[inline]
    pub fn assign_code_now(&mut self, d: DriverId, r: Range, code: u8) -> bool {
        self.assign_code_fast(d, Some(r), Time::ZERO, code, DelayMode::Transport)
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
        self.assign(d, target, &[(delay, bytes)], mode)
    }

    /// The common scalar byte case of `assign` in one pass: one element to a driver with nothing
    /// pending is kept as a `BQuick` (an unchanged zero-delay one is skipped as in `assign`).
    /// False when the general path must run (it also produces every error).
    #[inline]
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
            || info.direct
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
        if at == now && self.unchanged(sig, d, r, ValueRef::Bytes(bytes)) {
            let w = &mut self.st.waveforms[local];
            w.phantom.push(r);
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
        self.st.scheduled.push((at, d));
        true
    }

    /// The common vector case of `assign` in one pass: a zero-delay assignment of at most 64
    /// logic elements (`planes`, `len` elements) to a driver with nothing pending is kept as a
    /// `VQuick`, no list entry and no value buffer. False when the general path must run (it
    /// also produces every error).
    #[inline]
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
            if (info.direct && self.direct_entries(info.signal.0, info.slot))
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
            self.st.scheduled.push((at, d));
            return true;
        }
        if info.direct {
            let (sig, slot) = (info.signal.0, info.slot);
            // The driver's one-element entries keep the order: the general path moves them.
            if self.st.out.iter().any(|o| (o.sig, o.slot) == (sig, slot)) {
                return false;
            }
            self.st.outv.push(OutV {
                sig,
                slot,
                elem: r.offset,
                len,
                planes: planes.map(|p| p & yavs_logic::wide::nmask(len as usize)),
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
        self.st.scheduled.push((now, d));
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

    /// The process has a direct entry for `slot` of signal `sig` in this run (they commit at its
    /// end, so a delayed assignment must come after them: the general path orders it).
    fn direct_entries(&self, sig: u32, slot: u32) -> bool {
        self.st.out.iter().any(|o| (o.sig, o.slot) == (sig, slot))
            || self.st.outv.iter().any(|o| (o.sig, o.slot) == (sig, slot))
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
        self.st.scheduled.extend(new.iter().map(|t| (t.time, d)));
        self.st.waveforms[local].schedule(r, reject_from, new);
        Ok(())
    }

    /// Add `r` to the sensitivity of the coming wait.
    #[inline(always)]
    pub fn wait_on(&mut self, r: impl Into<SigRef>) -> Result<(), SimError> {
        let r = r.into();
        let range = self.sig(r)?.1;
        // Most waits name one signal: no room for four (the list becomes the previous one).
        if self.st.sens.capacity() == 0 {
            self.st.sens.reserve_exact(1);
        }
        self.st.sens.push((r.signal, range));
        Ok(())
    }

    /// `wait_on` of one range, when the last wait was on exactly that range, with no timeout,
    /// and nothing was added to this one: the commit keeps the registered waiter and `sens`
    /// stays empty. `false` (nothing done) otherwise; the caller then uses `wait_on`.
    #[inline]
    pub fn wait_on_again(&mut self, signal: SignalId, range: Range) -> bool {
        let st = &mut *self.st;
        if st.sens.is_empty()
            && st.deadline.is_none()
            && let [prev] = &st.sens_prev[..]
            && *prev == (signal, range)
        {
            st.again = true;
            return true;
        }
        false
    }

    /// The same wait as the last one (its sensitivity, no timeout), when that one had no
    /// timeout and nothing was added to this one: the commit keeps the registered waiters, as
    /// `wait_on_again` does for one range. `false` (nothing done) otherwise; the caller then
    /// names the wait with `wait_on`.
    #[inline]
    pub fn wait_same(&mut self) -> bool {
        let st = &mut *self.st;
        if st.sens.is_empty() && st.deadline.is_none() && !st.sens_prev.is_empty() {
            st.again = true;
            return true;
        }
        false
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
