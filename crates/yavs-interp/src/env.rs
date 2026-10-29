//! The 2019 `std.env` natives that need interpreter state: call paths (`get_call_path`,
//! `file_name`, `file_path`, `file_line`), the VHDL assert state (`GetVhdlAssertCount` and
//! friends) and `time_record` (#99).
//!
//! **Assert state and determinism (rule 3).** Counts, enables and the read severity are global,
//! but processes of one cycle may run in parallel. Each process therefore sees the state as of
//! the start of its cycle plus its own changes in this cycle; the changes of all processes are
//! applied in process order when the next cycle first touches the state, the order in which the
//! kernel commits their reports. `--threads 1` and `--threads N` see the same values.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use yavs_diag::{SourceMap, Span};
use yavs_ir::{Program, TyId, Width};
use yavs_kernel::{Ctx, Severity, Time};
use yavs_std::env::{self, DayOfWeek, TimeRecord};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::native::NativeFn;
use crate::textio::arg;

// ---- source locations ----

/// The design's source files, for call paths: path and line starts (byte offsets, as spans
/// count) per `FileId`.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    // Shared: a runtime is made per package initialisation and holds a copy of these.
    files: Vec<Option<Arc<File>>>,
}

/// A file's path and line starts.
type File = (String, Vec<u32>);

impl Sources {
    /// The files `prog`'s statements are in, taken from the session's source map.
    pub fn of(prog: &Program, map: &SourceMap) -> Sources {
        let mut s = Sources::default();
        s.add_from(prog, 0, map);
        s
    }

    /// Add the files of `prog`'s statements from statement `first` on.
    pub(crate) fn add_from(&mut self, prog: &Program, first: usize, map: &SourceMap) {
        for (_, st) in prog.stmts.iter().skip(first) {
            let id = st.span.file;
            if self.files.get(id.0 as usize).is_none_or(Option::is_none) {
                let f = map.get(id);
                if f.is_latin1() {
                    self.add(id, &f.path, f.bytes());
                } else {
                    // The map has the line starts already.
                    let i = id.0 as usize;
                    if self.files.len() <= i {
                        self.files.resize(i + 1, None);
                    }
                    self.files[i] = Some(Arc::new((f.path.clone(), f.line_starts().to_vec())));
                }
            }
        }
    }

    /// Add one file (tests build programs by hand).
    pub fn add(&mut self, file: yavs_diag::FileId, path: &str, bytes: &[u8]) {
        let i = file.0 as usize;
        if self.files.len() <= i {
            self.files.resize(i + 1, None);
        }
        let nl = bytes.iter().enumerate().filter(|(_, b)| **b == b'\n');
        let starts = std::iter::once(0)
            .chain(nl.map(|(i, _)| i as u32 + 1))
            .collect();
        self.files[i] = Some(Arc::new((path.to_string(), starts)));
    }

    /// File path of a span.
    pub(crate) fn path(&self, span: Span) -> Option<&str> {
        Some(&self.files.get(span.file.0 as usize)?.as_ref()?.0)
    }

    /// File path and 1-based line of a span.
    fn locate(&self, span: Span) -> Option<(&str, i64)> {
        let (path, starts) = &**self.files.get(span.file.0 as usize)?.as_ref()?;
        let line = starts.partition_point(|&s| s <= span.start);
        Some((path, line as i64))
    }
}

/// One element of a call path: the subprogram or process name (upper case, as sim1), the file
/// name, the absolute directory and the line. `to_string` gives sim1's form
/// `<path>/<file>:<line>:<NAME>` (`Machine::element_text`).
struct Element {
    name: String,
    file_name: String,
    file_path: String,
    line: i64,
}

impl Machine {
    /// The call path, innermost frame (the caller of the native) first.
    fn call_path(&self) -> R<Vec<Element>> {
        let prog = &self.rt.prog;
        let mut v = Vec::new();
        for fr in self.frames.iter().rev() {
            let f = prog
                .func(fr.func)
                .ok_or_else(|| ice(format!("no function {}", fr.func)))?;
            let Some(&loc) = f.loc.get(fr.pc as usize) else {
                continue;
            };
            // A function inlined at this instruction is an element of its own, its caller the
            // next one.
            let mut at = vec![(f.name.as_str(), loc)];
            let chain: Vec<_> = f.inlined_chain(fr.pc as usize).collect();
            if !chain.is_empty() {
                // (The instruction's function, then each caller outward with the statement of
                // the call.)
                let name = |id| {
                    prog.func(id)
                        .map(|g| g.name.as_str())
                        .ok_or_else(|| ice(format!("no function {id}")))
                };
                at = vec![(name(chain[0].callee)?, loc)];
                for (k, n) in chain.iter().enumerate() {
                    let outer = match chain.get(k + 1) {
                        Some(o) => name(o.callee)?,
                        None => f.name.as_str(),
                    };
                    at.push((outer, n.site));
                }
            }
            for (name, s) in at {
                let st = prog
                    .stmts
                    .get(s)
                    .ok_or_else(|| ice(format!("no statement {s:?}")))?;
                let Some((path, line)) = self.rt.sources.locate(st.span) else {
                    return rt_err(
                        "the call path needs the design's source files, which this simulation \
                         was not given (yavs_interp::Bindings::sources)",
                    );
                };
                let p = std::path::absolute(path).unwrap_or_else(|_| path.into());
                v.push(Element {
                    name: name.to_ascii_uppercase(),
                    file_name: p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    file_path: p
                        .parent()
                        .map(|d| d.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    line,
                });
            }
        }
        Ok(v)
    }

    fn here(&self) -> R<Element> {
        self.call_path()?
            .into_iter()
            .next()
            .ok_or_else(|| ice("call path of an empty frame stack"))
    }

    /// The four cells of a `call_path_element` at `a`: three line handles and the line.
    fn element_text(&mut self, a: u64) -> R<String> {
        let mut parts = Vec::new();
        for k in 0..3 {
            parts.push(self.line_get(add_addr(a, 8 * k)?)?.unwrap_or_default());
        }
        let line = self.load(add_addr(a, 24)?, Width::B8)? as i64;
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        Ok(format!(
            "{}{}{}:{}:{}",
            s(&parts[2]),
            std::path::MAIN_SEPARATOR,
            s(&parts[1]),
            line,
            s(&parts[0])
        ))
    }

    /// `to_string` of `n` elements at `data`, joined by `sep`.
    fn path_text(&mut self, data: u64, n: u64, sep: &[u8]) -> R<Vec<u8>> {
        let mut out = Vec::new();
        for k in 0..n {
            if k > 0 {
                out.extend_from_slice(sep);
            }
            let e = self.element_text(add_addr(data, k * ELEMENT_BYTES)?)?;
            out.extend(e.bytes());
        }
        Ok(out)
    }
}

/// `call_path_element` is `(name, file_name, file_path: line; file_line: positive)`: three
/// handles and an integer, 8 bytes each.
const ELEMENT_BYTES: u64 = 32;

fn get_call_path(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    let path = m.call_path()?;
    // A heap array object `[0 to n - 1][elements]`; the line fields are handles and the line
    // numbers never decode as one (tag 0), so the object is a handle vector for the GC.
    let mut obj: Vec<u8> = [0i64, path.len() as i64 - 1, 0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    for e in &path {
        for s in [&e.name, &e.file_name, &e.file_path] {
            let h = m.new_line(0, s.as_bytes())?;
            obj.extend(h.to_le_bytes());
        }
        obj.extend(e.line.to_le_bytes());
    }
    m.alloc_line_vector(0, obj).map(Some)
}

/// The data address and element count of an array argument (a fat pair).
fn array_arg(m: &mut Machine, fat: u64) -> R<(u64, u64)> {
    let data = m.load(fat, Width::B8)?;
    let dope = m.load(add_addr(fat, 8)?, Width::B8)?;
    let l = m.load(dope, Width::B8)? as i64;
    let r = m.load(add_addr(dope, 8)?, Width::B8)? as i64;
    let d = m.load(add_addr(dope, 16)?, Width::B8)?;
    let n = yavs_ir::dim_len(l, r, d == 1).ok_or_else(|| ice("call path length"))?;
    Ok((data, n))
}

/// A record argument: lowering passes composites to natives as a fat pair (its data address
/// first).
pub(crate) fn record_arg(m: &mut Machine, fat: u64) -> R<u64> {
    m.load(fat, Width::B8)
}

fn path_element_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let r = record_arg(m, arg(a, 0)?)?;
    let s = m.element_text(r)?;
    m.ret_string(s.as_bytes()).map(Some)
}

fn path_vector_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let (data, n) = array_arg(m, arg(a, 0)?)?;
    let sep = m.fat_bytes(arg(a, 1)?)?;
    let s = m.path_text(data, n, &sep)?;
    m.ret_string(&s).map(Some)
}

fn path_ptr_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let h = arg(a, 0)?;
    let sep = m.fat_bytes(arg(a, 1)?)?;
    if h == 0 {
        return rt_err("null access dereference");
    }
    let n = m.line_vector_handles(h)?.len() as u64 / 4;
    let (hp, space) = m.heap_of(h)?;
    let i = hp
        .resolve(h)
        .map_err(|_| Fault::Runtime("call path was already deallocated".into()))?;
    let data = crate::exec::mk_addr(space, i, 24)?;
    let s = m.path_text(data, n, &sep)?;
    m.ret_string(&s).map(Some)
}

macro_rules! here_natives {
    ($($name:ident => |$e:ident, $m:ident| $body:expr),* $(,)?) => {$(
        fn $name(
            $m: &mut Machine,
            _: &mut Option<&mut Ctx<'_>>,
            _: Option<TyId>,
            _: &[u64],
        ) -> R<Option<u64>> {
            let $e = $m.here()?;
            $body.map(Some)
        }
    )*};
}

here_natives! {
    file_name_line => |e, m| m.new_line(0, e.file_name.as_bytes()),
    file_name_string => |e, m| m.ret_string(e.file_name.as_bytes()),
    file_path_line => |e, m| m.new_line(0, e.file_path.as_bytes()),
    file_path_string => |e, m| m.ret_string(e.file_path.as_bytes()),
    file_line_positive => |e, _m| R::Ok(e.line as u64),
    file_line_string => |e, m| m.ret_string(e.line.to_string().as_bytes()),
}

// ---- VHDL assert state ----

#[derive(Clone, Debug)]
struct AssertState {
    counts: [u64; 4],
    enabled: [bool; 4],
    read_severity: Severity,
}

impl Default for AssertState {
    fn default() -> AssertState {
        // The read severity starts at ERROR, as sim1 (the LRM gives no initial value).
        AssertState {
            counts: [0; 4],
            enabled: [true; 4],
            read_severity: Severity::Error,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    /// `n` reports of the severity (consecutive counts are merged: `Asserts::with` replays the
    /// log on every report, so a log with one entry per report is quadratic).
    Count(Severity, u64),
    Clear,
    Enable(Severity, bool),
    ReadSeverity(Severity),
}

impl AssertState {
    fn apply(&mut self, op: Op) {
        match op {
            Op::Count(s, n) => self.counts[s as usize] += n,
            Op::Clear => self.counts = [0; 4],
            Op::Enable(s, b) => self.enabled[s as usize] = b,
            Op::ReadSeverity(s) => self.read_severity = s,
        }
    }
}

/// Reports made while elaborating, per severity. Elaboration evaluates functions in throwaway
/// runtimes, so the count lives outside them; the CLI adds it to the exit status.
// ponytail: process-wide and not cleared by ClearVhdlAssert; carry it in the elaboration result
// if a second elaboration in one process (the in-process tests) or a clear must see it.
static ELAB_COUNTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// The reports of severity `s` made during elaboration so far in this process.
pub fn elaboration_reports(s: Severity) -> u64 {
    ELAB_COUNTS[s as usize].load(Ordering::Relaxed)
}

/// The line of a report made while no kernel runs.
pub(crate) fn print_elab(s: Severity, text: &str) {
    use std::io::Write;
    let name = ["Note", "Warning", "Error", "Failure"][s as usize];
    let mut o = std::io::stderr().lock();
    let _ = writeln!(o, "** {name}: (init): {text}");
}

/// Print and count again the reports a package initialisation made (`SharedState::replay_init_reports`).
pub(crate) fn replay_elab(reports: &[(u8, String)]) {
    for (s, text) in reports {
        ELAB_COUNTS[*s as usize].fetch_add(1, Ordering::Relaxed);
        print_elab(sev_of(*s), text);
    }
}

fn sev_of(i: u8) -> Severity {
    match i {
        0 => Severity::Note,
        1 => Severity::Warning,
        2 => Severity::Error,
        _ => Severity::Failure,
    }
}

/// The assert state of a simulation (module docs).
#[derive(Default)]
pub(crate) struct Asserts(Mutex<Shared>);

#[derive(Default)]
struct Shared {
    base: AssertState,
    cycle: Option<(Time, u32)>,
    /// This cycle's changes per process id, applied in process order.
    logs: BTreeMap<u32, Vec<Op>>,
}

impl Asserts {
    /// Run `f` on the state as `ctx`'s process sees it, then log `op` for it.
    fn with<T>(&self, ctx: &Ctx<'_>, op: Option<Op>, f: impl FnOnce(&AssertState) -> T) -> T {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = (ctx.now(), ctx.delta());
        if g.cycle != Some(now) {
            let logs = std::mem::take(&mut g.logs);
            for op in logs.into_values().flatten() {
                g.base.apply(op);
            }
            g.cycle = Some(now);
        }
        let pid = ctx.process().0;
        let mut st = g.base.clone();
        for &o in g.logs.get(&pid).into_iter().flatten() {
            st.apply(o);
        }
        let r = f(&st);
        if let Some(op) = op {
            let log = g.logs.entry(pid).or_default();
            // Counts commute with each other: fold into the trailing run of counts.
            let merged = match op {
                Op::Count(s, n) => log
                    .iter_mut()
                    .rev()
                    .map_while(|o| match o {
                        Op::Count(s2, n2) => Some((s2, n2)),
                        _ => None,
                    })
                    .find(|(s2, _)| **s2 == s)
                    .map(|(_, n2)| *n2 += n)
                    .is_some(),
                _ => false,
            };
            if !merged {
                log.push(op);
            }
        }
        r
    }

    /// The counts per severity with every logged change applied (the end of the run).
    pub(crate) fn counts(&self) -> [u64; 4] {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = g.base.clone();
        for &op in g.logs.values().flatten() {
            st.apply(op);
        }
        st.counts
    }

    /// Take `from`'s state as this one's base (`Runtime::inherit`).
    pub(crate) fn inherit(&self, from: &Asserts) {
        let base = from
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .base
            .clone();
        self.0.lock().unwrap_or_else(|e| e.into_inner()).base = base;
    }

    /// A report during elaboration (no kernel yet): counted in `ELAB_COUNTS`, false when that
    /// severity is disabled.
    pub(crate) fn report_elab(&self, s: Severity) -> bool {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let on = g.base.enabled[s as usize];
        if on {
            ELAB_COUNTS[s as usize].fetch_add(1, Ordering::Relaxed);
        }
        on
    }

    /// A report or assertion violation of severity `s`: false when that severity is disabled
    /// (not shown, not counted, as sim1).
    pub(crate) fn report(&self, ctx: &Ctx<'_>, s: Severity) -> bool {
        let on = self.with(ctx, None, |st| st.enabled[s as usize]);
        if on {
            self.with(ctx, Some(Op::Count(s, 1)), |_| ());
        }
        on
    }
}

fn sev(v: u64) -> R<Severity> {
    Ok(match v {
        0 => Severity::Note,
        1 => Severity::Warning,
        2 => Severity::Error,
        3 => Severity::Failure,
        _ => return Err(ice(format!("severity {v}"))),
    })
}

fn kctx<'a, 'c>(ctx: &'a mut Option<&mut Ctx<'c>>) -> R<&'a mut Ctx<'c>> {
    match ctx.as_deref_mut() {
        Some(c) => Ok(c),
        None => rt_err("simulation-only native called during elaboration"),
    }
}

/// An assert-state native: `op` from the arguments, then the result from the state seen
/// before it.
fn assert_native(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    a: &[u64],
    op: impl FnOnce(&[u64]) -> R<Option<Op>>,
    f: impl FnOnce(&AssertState, &[u64]) -> R<Option<u64>>,
) -> R<Option<u64>> {
    let c = kctx(ctx)?;
    let op = op(a)?;
    let rt = m.rt.clone();
    rt.asserts.with(c, op, |st| f(st, a))
}

/// Warning, error and failure: what the no-argument forms count (as sim1).
const VIOLATIONS: [usize; 3] = [1, 2, 3];

macro_rules! assert_natives {
    ($($name:ident => ($op:expr, $f:expr)),* $(,)?) => {$(
        fn $name(
            m: &mut Machine,
            ctx: &mut Option<&mut Ctx<'_>>,
            _: Option<TyId>,
            a: &[u64],
        ) -> R<Option<u64>> {
            assert_native(m, ctx, a, $op, $f)
        }
    )*};
}

fn none(_: &[u64]) -> R<Option<Op>> {
    Ok(None)
}

assert_natives! {
    is_failed => (none, |st, _| Ok(Some(u64::from(VIOLATIONS.iter().any(|&k| st.counts[k] > 0))))),
    is_failed_level => (none, |st, a| Ok(Some(u64::from(st.counts[sev(arg(a, 0)?)? as usize] > 0)))),
    count => (none, |st, _| Ok(Some(VIOLATIONS.iter().map(|&k| st.counts[k]).sum()))),
    count_level => (none, |st, a| Ok(Some(st.counts[sev(arg(a, 0)?)? as usize]))),
    clear => (|_| Ok(Some(Op::Clear)), |_, _| Ok(None)),
    get_enable => (none, |st, a| Ok(Some(u64::from(st.enabled[sev(arg(a, 0)?)? as usize])))),
    get_read_severity => (none, |st, _| Ok(Some(st.read_severity as u64))),
    set_read_severity => (|a| Ok(Some(Op::ReadSeverity(sev(arg(a, 0)?)?))), |_, _| Ok(None)),
    get_format => (none, |_, _| Ok(None)),
}

/// `SetVhdlAssertEnable(Enable)`: all four severities, logged as four changes.
fn set_enable_all(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let b = arg(a, 0)? != 0;
    let c = kctx(ctx)?;
    for s in [
        Severity::Note,
        Severity::Warning,
        Severity::Error,
        Severity::Failure,
    ] {
        m.rt.asserts.with(c, Some(Op::Enable(s, b)), |_| ());
    }
    Ok(None)
}

fn set_enable(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let (s, b) = (sev(arg(a, 0)?)?, arg(a, 1)? != 0);
    let c = kctx(ctx)?;
    m.rt.asserts.with(c, Some(Op::Enable(s, b)), |_| ());
    Ok(None)
}

/// `GetVhdlAssertFormat`: always the empty string (the default format).
fn get_format_string(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    t: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    get_format(m, ctx, t, a)?;
    m.ret_string(b"").map(Some)
}

/// `SetVhdlAssertFormat`: only the empty format (the default) is supported.
fn set_format(m: &mut Machine, a: &[u64]) -> R<()> {
    if !m.fat_bytes(arg(a, 1)?)?.is_empty() {
        return rt_err("SetVhdlAssertFormat with a non-empty format is not implemented yet (#99)");
    }
    Ok(())
}

fn set_format_proc(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    set_format(m, a)?;
    Ok(None)
}

fn set_format_valid(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    set_format(m, a)?;
    m.store(arg(a, 2)?, Width::B1, 1)?;
    Ok(None)
}

// ---- time_record ----

/// `time_record` in memory: seven integers, the `dayofweek` byte and `dayofyear`, naturally
/// aligned (`yavs_ir::Ty::layout`).
const TREC_BYTES: u64 = 72;
const WEEKDAY_AT: usize = 56;
const DAYOFYEAR_AT: usize = 64;

fn trec_err(e: env::TimeError) -> Fault {
    Fault::Runtime(e.to_string())
}

fn read_trec(m: &mut Machine, fat: u64) -> R<TimeRecord> {
    let r = record_arg(m, fat)?;
    let b = m.rd(r, TREC_BYTES)?;
    let w = |k: usize| i64::from_le_bytes(b[k..k + 8].try_into().unwrap_or([0; 8]));
    const DAYS: [DayOfWeek; 7] = [
        DayOfWeek::Sunday,
        DayOfWeek::Monday,
        DayOfWeek::Tuesday,
        DayOfWeek::Wednesday,
        DayOfWeek::Thursday,
        DayOfWeek::Friday,
        DayOfWeek::Saturday,
    ];
    let weekday = *DAYS
        .get(b[WEEKDAY_AT] as usize)
        .ok_or_else(|| ice("dayofweek position out of range"))?;
    Ok(TimeRecord {
        microsecond: w(0),
        second: w(8),
        minute: w(16),
        hour: w(24),
        day: w(32),
        month: w(40),
        year: w(48),
        weekday,
        dayofyear: w(DAYOFYEAR_AT),
    })
}

/// A `time_record` result: a fat pair over a temporary (the record needs no descriptor).
fn ret_trec(m: &mut Machine, t: TimeRecord) -> R<Option<u64>> {
    let mut b = vec![0u8; TREC_BYTES as usize];
    for (k, v) in [
        t.microsecond,
        t.second,
        t.minute,
        t.hour,
        t.day,
        t.month,
        t.year,
    ]
    .into_iter()
    .enumerate()
    {
        b[k * 8..k * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    b[WEEKDAY_AT] = t.weekday as u8;
    b[DAYOFYEAR_AT..].copy_from_slice(&t.dayofyear.to_le_bytes());
    let data = m.temp_alloc(TREC_BYTES)?;
    m.put(data, &b)?;
    let fat = m.temp_alloc(16)?;
    m.store(fat, Width::B8, data)?;
    m.store(add_addr(fat, 8)?, Width::B8, data)?;
    Ok(Some(fat))
}

fn now_epoch() -> R<f64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .map_err(|_| Fault::Runtime("system clock is before 1970".into()))
}

fn real(a: &[u64], i: usize) -> R<f64> {
    arg(a, i).map(f64::from_bits)
}

macro_rules! trec_natives {
    ($($name:ident => |$m:ident, $a:ident| $body:expr),* $(,)?) => {$(
        fn $name(
            $m: &mut Machine,
            _: &mut Option<&mut Ctx<'_>>,
            _: Option<TyId>,
            $a: &[u64],
        ) -> R<Option<u64>> {
            $body
        }
    )*};
}

trec_natives! {
    localtime_now => |m, _a| ret_trec(m, env::localtime(now_epoch()?).map_err(trec_err)?),
    gmtime_now => |m, _a| ret_trec(m, env::gmtime(now_epoch()?).map_err(trec_err)?),
    localtime_real => |m, a| ret_trec(m, env::localtime(real(a, 0)?).map_err(trec_err)?),
    gmtime_real => |m, a| ret_trec(m, env::gmtime(real(a, 0)?).map_err(trec_err)?),
    epoch_trec => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        Ok(Some(env::epoch(&t).map_err(trec_err)?.to_bits()))
    },
    localtime_trec => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        ret_trec(m, env::localtime_of(&t).map_err(trec_err)?)
    },
    gmtime_trec => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        ret_trec(m, env::gmtime_of(&t).map_err(trec_err)?)
    },
    add_trec_real => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        ret_trec(m, env::add_seconds(&t, real(a, 1)?).map_err(trec_err)?)
    },
    add_real_trec => |m, a| {
        let t = read_trec(m, arg(a, 1)?)?;
        ret_trec(m, env::add_seconds(&t, real(a, 0)?).map_err(trec_err)?)
    },
    sub_trec_real => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        ret_trec(m, env::sub_seconds(&t, real(a, 1)?).map_err(trec_err)?)
    },
    // `DELTA - TREC`: the record moved back by DELTA, like `TREC - DELTA` (the LRM gives no
    // other meaning to a record subtracted from a number).
    sub_real_trec => |m, a| {
        let t = read_trec(m, arg(a, 1)?)?;
        ret_trec(m, env::sub_seconds(&t, real(a, 0)?).map_err(trec_err)?)
    },
    diff_trec => |m, a| {
        let x = read_trec(m, arg(a, 0)?)?;
        let y = read_trec(m, arg(a, 1)?)?;
        Ok(Some(env::diff_seconds(&x, &y).map_err(trec_err)?.to_bits()))
    },
    trec_string => |m, a| {
        let t = read_trec(m, arg(a, 0)?)?;
        let d = arg(a, 1)? as i64;
        let d = u32::try_from(d)
            .ok()
            .filter(|d| *d <= 6)
            .ok_or_else(|| Fault::Runtime(format!("value {d} outside of range 0 to 6 for parameter FRAC_DIGITS")))?;
        let s = env::time_record_to_string(&t, d).map_err(trec_err)?;
        m.ret_string(&s).map(Some)
    },
}

/// The implementation of a `std.env` native defined here.
pub(crate) fn lookup(symbol: &str) -> Option<NativeFn> {
    let s = symbol.strip_prefix("yavs.env.")?;
    Some(match s {
        "get_call_path.void.r_call_path_vector_ptr" => get_call_path,
        "to_string.call_path_element.r_string" => path_element_string,
        "to_string.call_path_vector_string.r_string" => path_vector_string,
        "to_string.call_path_vector_ptr_string.r_string" => path_ptr_string,
        "file_name.void.r_line" => file_name_line,
        "file_name.void.r_string" => file_name_string,
        "file_path.void.r_line" => file_path_line,
        "file_path.void.r_string" => file_path_string,
        "file_line.void.r_positive" => file_line_positive,
        "file_line.void.r_string" => file_line_string,
        "isvhdlassertfailed.void.r_boolean" => is_failed,
        "isvhdlassertfailed.severity_level.r_boolean" => is_failed_level,
        "getvhdlassertcount.void.r_natural" => count,
        "getvhdlassertcount.severity_level.r_natural" => count_level,
        "clearvhdlassert.void" => clear,
        "setvhdlassertenable.boolean" => set_enable_all,
        "setvhdlassertenable.severity_level_boolean" => set_enable,
        "getvhdlassertenable.severity_level.r_boolean" => get_enable,
        "setvhdlassertformat.severity_level_string" => set_format_proc,
        "setvhdlassertformat.severity_level_string_boolean" => set_format_valid,
        "getvhdlassertformat.severity_level.r_string" => get_format_string,
        "setvhdlreadseverity.severity_level" => set_read_severity,
        "getvhdlreadseverity.void.r_severity_level" => get_read_severity,
        "localtime.void.r_time_record" => localtime_now,
        "gmtime.void.r_time_record" => gmtime_now,
        "localtime.real.r_time_record" => localtime_real,
        "gmtime.real.r_time_record" => gmtime_real,
        "epoch.time_record.r_real" => epoch_trec,
        "localtime.time_record.r_time_record" => localtime_trec,
        "gmtime.time_record.r_time_record" => gmtime_trec,
        "add.time_record_real.r_time_record" => add_trec_real,
        "add.real_time_record.r_time_record" => add_real_trec,
        "sub.time_record_real.r_time_record" => sub_trec_real,
        "sub.real_time_record.r_time_record" => sub_real_trec,
        "sub.time_record_time_record.r_real" => diff_trec,
        "to_string.time_record_integer.r_string" => trec_string,
        _ => return None,
    })
}
