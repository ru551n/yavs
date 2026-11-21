//! Cycle mode (`yavs run --cycle`): the processes that can run cycle-based form one region,
//! run by one kernel process on a static schedule with compiled code; everything else stays in
//! the event kernel. Read `AGENTS.md` first.

mod check;
mod codegen;
mod dep;
mod lower;
mod region;
mod rt;
mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yavs_diag::Diagnostic;
use yavs_elab::{Elaborated, LeafId, Region, RegionDriver};
use yavs_kernel::{EDGE_LEVEL, EdgeTrigger, Range, SimError};
use yavs_sem::Db;

pub use codegen::{set_cache, set_threads};
use lower::{Kind, Op, Ref};
pub use region::set_exact_meta;
pub use store::analysis_cache;

/// Codes of cycle mode. Under `--cycle-strict` every reason a process below the top-level unit
/// stays out is an error with its code; under `--cycle` the report gives the same codes.
pub mod codes {
    /// The process is in the subset, but cycle mode does not implement something it needs yet.
    pub const LEFT_OUT: &str = "E4101";
    /// `--cycle` with an option that observes or changes signals inside the region.
    pub const NOT_WITH: &str = "E4102";
    /// The file of `--cycle-report=FILE` cannot be written.
    pub const REPORT_FILE: &str = "E4103";
    /// An environment variable of cycle mode with a value it does not take.
    pub const ENV_VALUE: &str = "E4104";
    /// Asserts and reports of severity error or failure that cycle-based processes drop (A41).
    pub const DROPPED_STOPS: &str = "W4101";
    /// A wait other than the one `wait until <edge>;` of a clocked process.
    pub const WAIT: &str = "E4110";
    /// `after`, `transport`, `reject`, a waveform of several elements.
    pub const DELAY: &str = "E4111";
    /// Signal history: `'event` outside the clock's edge test, `'last_value`, `'stable`, ...
    pub const HISTORY: &str = "E4112";
    /// A real or physical (time) value computed at run time.
    pub const NOT_TWO_STATE: &str = "E4113";
    /// Shared variables, methods of protected objects, allocators and access values.
    pub const TESTBENCH: &str = "E4114";
    /// Postponed processes, guarded blocks, signals and assignments.
    pub const GUARDED: &str = "E4115";
    /// `force` and `release`.
    pub const FORCE: &str = "E4116";
    /// A call the analysis cannot follow: foreign, no body, or (combinational) changing state
    /// outside the subprogram.
    pub const CALL: &str = "E4117";
    /// A `'Z'` value.
    pub const HIGH_Z: &str = "E4118";
    /// A combinational process that does not assign a driven element on every path.
    pub const LATCH: &str = "E4119";
    /// A variable of a combinational process read before it is assigned.
    pub const STATE: &str = "E4120";
    /// A combinational process reading a signal its sensitivity list does not name.
    pub const SENSITIVITY: &str = "E4121";
    /// A loop in the combinational logic, element by element.
    pub const LOOP: &str = "E4122";
    /// A clocked process of a shape cycle mode does not take.
    pub const SHAPE: &str = "E4123";
    /// A signal it drives has another source: another process, a resolution, a guard.
    pub const SHARED: &str = "E4124";
    /// A clock made inside the region: its combinational maker, or the users of a register-made
    /// one.
    pub const CLOCK_MADE: &str = "E4125";
    /// A construct yavs does not follow far enough to show the process is logic (a limitation).
    pub const ANALYSIS: &str = "E4126";
    /// A process outside the region reads the history of a signal the process drives
    /// (`'active`, `'last_active`, `'last_event`, `'last_value`, `'transaction`, `'stable`,
    /// `'quiet`, `'delayed`).
    pub const HISTORY_READ: &str = "E4127";
    /// `force` or `release` somewhere in the design: no process joins (a limitation).
    pub const FORCED: &str = "E4128";
    /// Logic feeding a process outside whose result depends on the delta cycles its inputs
    /// change in (the run-count cone).
    pub const DELTA_FANIN: &str = "E4129";
    /// Logic between registers of clocks whose edges come in different delta cycles.
    pub const CROSS_DELTA: &str = "E4130";
    /// Logic that computes part of a clock from what the region drives.
    pub const CLOCK_LOGIC: &str = "E4131";
    /// A register reset by another register, driving a clock or asynchronous reset (a pulse).
    pub const PULSE: &str = "E4132";
    /// A memory written in a form cycle mode does not take yet (a limitation).
    pub const MEMORY: &str = "E4133";
    /// A signal driven partly inside the region and partly outside (a limitation).
    pub const SPLIT: &str = "E4134";
    /// A driven signal that also has a port or constant source (a limitation).
    pub const PORT_SOURCE: &str = "E4135";
    /// An assert of the top-level unit reading a signal the region does not drive alone.
    pub const CHECK_OUTSIDE: &str = "E4136";
    /// Under `--cycle-exact-meta`: a metavalue the testbench would see as '0' or '1'.
    pub const META_OUT: &str = "E4137";
    /// An external name (a limitation).
    pub const EXTERNAL: &str = "E4138";
    /// A loop without bounds known during elaboration, too large, or with a jump out of an
    /// outer loop.
    pub const LOOP_BOUND: &str = "E4139";
    /// A reason no design can reach (the analysis or an earlier check rules it out): a bug in
    /// yavs, kept as a reason so the process stays in the event kernel (`check::bug`, `lower::bug`).
    pub const INTERNAL: &str = "E9999";

    /// Every code of a reason a process stays out: (code, report title, why it cannot be
    /// cycle-based, how to fix it, whether it is a limitation of yavs rather than the design).
    pub(crate) const REASONS: &[(&str, &str, &str, &str, bool)] = &[
        (
            INTERNAL,
            "an internal error in yavs",
            "yavs reached a state its own earlier checks rule out",
            "please report this as a bug in yavs, with the design; run with `--cycle` to leave this process in the event simulation meanwhile",
            true,
        ),
        (
            LEFT_OUT,
            "a construct cycle mode does not implement yet",
            "yavs does not implement this construct in cycle-based code yet",
            "run with `--cycle` to leave this process in the event simulation, or write the construct another way",
            true,
        ),
        (
            WAIT,
            "a wait that is not the clock edge",
            "a cycle-based process runs once per clock edge or input change, so its only wait can be `wait until <edge>;` as its first statement",
            "write it as a clocked process (`if rising_edge(clk) then ... end if;`) or as a combinational process with a sensitivity list; a model with waits belongs in the testbench",
            false,
        ),
        (
            DELAY,
            "a delay, `transport`, `reject` or a waveform",
            "cycle-based code has no time inside a clock cycle, so it cannot keep a delay",
            "assign without a delay (`q <= d;`) in the design; keep delays in testbench models",
            false,
        ),
        (
            HISTORY,
            "signal history",
            "cycle-based code keeps only the present value of each signal, not its history",
            "use an edge test of the process's own clock, or keep the old value in a register of your own",
            false,
        ),
        (
            NOT_TWO_STATE,
            "a real or time value computed at run time",
            "cycle-based code computes logic values, integers and enumerations only",
            "use an integer or a constant (constant expressions are computed during elaboration), or move the model into the testbench",
            false,
        ),
        (
            TESTBENCH,
            "a shared variable, protected object or access value",
            "cycle-based code has no dynamic memory and no state shared between processes",
            "pass values between processes through signals; keep shared variables, protected objects and access values in the testbench",
            false,
        ),
        (
            GUARDED,
            "a postponed process or a guarded signal",
            "cycle mode has no end of a time step and no guarded signals",
            "use a plain process and an ordinary signal with one driver",
            false,
        ),
        (
            FORCE,
            "`force` or `release`",
            "`force` and `release` change a signal from outside its drivers, which cycle-based code does not see",
            "force signals from the testbench, not from a process of the design",
            false,
        ),
        (
            CALL,
            "a call without a VHDL body or with side effects",
            "cycle-based code inlines every call, so it needs the subprogram's VHDL body and must see everything the call changes",
            "give the subprogram a VHDL body; in a combinational process, pass what it changes as parameters",
            false,
        ),
        (
            HIGH_Z,
            "a `'Z'` value",
            "cycle-based code uses two-state values, which have no high impedance",
            "keep tri-state buses in the testbench or at the top of the design, or drive '0' and '1' with a separate enable signal",
            false,
        ),
        (
            LATCH,
            "a latch",
            "a signal that is not assigned on every path keeps its old value, which is state, not combinational logic",
            "assign it on every path, for example with a default assignment at the start of the process",
            false,
        ),
        (
            STATE,
            "a variable read before it is assigned",
            "a variable read before it is assigned keeps its value from the previous run, which is state, not combinational logic",
            "assign the variable at the start of the process, before any read",
            false,
        ),
        (
            SENSITIVITY,
            "an incomplete sensitivity list",
            "the event simulation does not run the process when that signal changes, so its result is not a function of what it reads, and cycle-based code would compute another result",
            "use `process (all)`, or name every signal the process reads in its sensitivity list",
            false,
        ),
        (
            LOOP,
            "a combinational loop",
            "logic in which an element depends on itself has no single settled value, so it cannot be put in a static order",
            "break the loop with a register, or split the signal so that no element depends on itself",
            false,
        ),
        (
            SHAPE,
            "a clocked process of another shape",
            "a clocked process runs cycle-based only in the clock-edge template",
            "write it as `if rising_edge(clk) then ... end if;`, optionally with an asynchronous reset branch first (`if rst = '1' then ... elsif rising_edge(clk) then`), nothing else in the process, and the clock and reset in its sensitivity list",
            false,
        ),
        (
            SHARED,
            "a signal with another source",
            "in cycle-based code every signal element has exactly one source, its process",
            "drive each element from one process, or merge the sources into one process",
            false,
        ),
        (
            CLOCK_MADE,
            "a clock made inside the design",
            "cycle-based code runs on edges of clocks that come from outside the region; a clock made inside it would change between those edges",
            "make the clock in the testbench, or use a clock enable on the main clock instead of a gated or divided clock",
            false,
        ),
        (
            ANALYSIS,
            "a construct yavs does not follow yet",
            "yavs cannot follow this construct far enough yet to show that the process is clocked or combinational logic",
            "run with `--cycle` to leave this process in the event simulation, or write the construct more simply",
            true,
        ),
        (
            HISTORY_READ,
            "its history is read outside the region",
            "a process outside the region reads the history of a signal this process drives, and cycle-based code keeps no history of the signals it drives",
            "let the reader keep its own copy of the old value, or use an edge test instead of `'last_value` or `'last_event`",
            false,
        ),
        (
            FORCED,
            "`force` or `release` elsewhere in the design",
            "a `force` or `release` the design can run changes a signal this process drives, which cycle-based code would not see; when yavs cannot tell which signal a force changes (a signal parameter), it keeps every process in the event simulation",
            "run with `--cycle` to leave this process in the event simulation, or drive the signal from the testbench instead of forcing it; force a signal through an external name or by name rather than through a signal parameter",
            true,
        ),
        (
            DELTA_FANIN,
            "logic feeding a process that depends on delta cycles",
            "the event simulation changes this logic over several delta cycles and the reader outside the region sees each step; cycle-based code changes it in one step, so the reader would see other values or run another number of times",
            "fix the reader so that it can join the region too (a complete sensitivity list, no latch, no variable read before it is assigned), or make it a clocked process",
            false,
        ),
        (
            CROSS_DELTA,
            "logic between clocks whose edges come in different delta cycles",
            "the edges of the two clocks come in different delta cycles of one time step: the event simulation samples this logic before it settles, cycle-based code would hand over the settled value",
            "clock both registers from one clock, or make the second clock a plain copy of the first (`clk_b <= clk_a;` counts as the same clock), not a gated or computed one",
            false,
        ),
        (
            CLOCK_LOGIC,
            "logic in a clock path",
            "cycle-based code gives its outputs one delta cycle after its inputs change, where the event simulation takes one delta cycle per level of logic, so the clock edge would come early",
            "make the clock in the testbench, or use a clock enable instead of logic on the clock",
            false,
        ),
        (
            PULSE,
            "a register reset by another register that drives a clock or reset",
            "the event simulation can change this register at the clock edge and again one delta cycle later, and that pulse clocks or resets another process; cycle-based code settles both at once and gives no pulse",
            "use a synchronous reset for this register, or a reset that does not come from another register",
            false,
        ),
        (
            MEMORY,
            "a memory written in a form cycle mode does not take yet",
            "yavs keeps a large memory in cycle-based code only when it is written at run-time indices in the clock-edge branch and read inside the region",
            "write the memory only in the clock-edge branch at run-time indices and read it only inside the design, or run with `--cycle`",
            true,
        ),
        (
            SPLIT,
            "a signal driven partly outside the region",
            "yavs does not split one signal between cycle-based code and the event simulation yet",
            "drive all elements of the signal from processes that can run cycle-based, or split it into two signals",
            true,
        ),
        (
            PORT_SOURCE,
            "a signal with a port or constant source",
            "the signal also gets its value from a port or a constant, which yavs does not model in cycle-based code yet",
            "drive the signal from one process only, or run with `--cycle`",
            true,
        ),
        (
            CHECK_OUTSIDE,
            "an assert reading a signal the region does not drive alone",
            "the assert must see the event simulation's values of that signal, so its process stays where those values are",
            "nothing to fix: the check runs in the event simulation as written",
            false,
        ),
        (
            META_OUT,
            "a metavalue the testbench would see as '0' or '1'",
            "under `--cycle-exact-meta` the testbench must see the metavalues the event simulation gives it, and cycle-based code gives only '0' and '1'",
            "run without `--cycle-exact-meta`, or assign '0' or '1' instead of the metavalue",
            false,
        ),
        (
            EXTERNAL,
            "an external name",
            "yavs does not follow external names in cycle-based code yet",
            "pass the signal through a port, or run with `--cycle` to leave this process in the event simulation",
            true,
        ),
        (
            LOOP_BOUND,
            "a loop cycle mode cannot unroll",
            "cycle-based code unrolls every loop, so it needs bounds known during elaboration, a size it can unroll, and `exit` and `next` of the innermost loop only",
            "give the loop bounds known during elaboration (constants or generics), split a very large loop, and use `exit` and `next` only for the innermost loop",
            false,
        ),
    ];

    /// The row of `code` in [`REASONS`].
    pub(crate) fn reason(
        code: &str,
    ) -> Option<&'static (&'static str, &'static str, &'static str, &'static str, bool)> {
        REASONS.iter().find(|r| r.0 == code)
    }
}

/// What the command line asked of cycle mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `--cycle`: leave out what does not fit, say so in the summary line.
    Run,
    /// `--cycle-strict`: what does not fit below the top-level unit is an error.
    Strict,
    /// `--cycle-report`: the region report only; nothing is compiled or run.
    Report,
}

/// What cycle mode decided for a design: the region report and the region to build.
pub struct Plan {
    /// The region report: what joined and what stays in the event kernel, with reasons
    /// ([`Mode::Report`] only, else empty).
    pub report: String,
    /// One line for `--cycle`: how many processes joined (ends with a newline).
    pub summary: String,
    /// `--cycle` and `--cycle-strict`: the warning that asserts and reports of severity error or
    /// failure are dropped (W4101), printed once after the summary.
    pub warning: Option<Diagnostic>,
    /// None under [`Mode::Report`] and when no process joined.
    pub region: Option<Region>,
    /// What the region read as 0 for a metavalue, for [`Metas::summary`] after the run.
    pub metas: Arc<Metas>,
    /// The report path of the normal mode, which the caller sets once the run's runtime exists:
    /// the asserts and reports of top-level members (A19) go through it (the assert counts of
    /// `std.env`, the exit status, the enabled severities), then out as the member's.
    pub reporter: Reporter,
}

/// The normal mode's report path: the text to report for a severity and message, or `None` when
/// the severity is disabled (`yavs_interp::Runtime::assert_report`).
pub type ReportFn =
    dyn Fn(&yavs_kernel::Ctx<'_>, yavs_kernel::Severity, String) -> Option<String> + Send + Sync;
pub type Reporter = Arc<std::sync::OnceLock<Arc<ReportFn>>>;

/// A short name of what a code stands for, for the headings of the region report.
fn title(code: &str) -> &'static str {
    codes::reason(code).map_or("", |r| r.1)
}

/// Elements the region read as 0 because they held a metavalue (`'U'`, `'X'`, `'Z'`, `'W'`,
/// `'-'`): at initialisation (signals and variables without an initial value included), and at
/// the inputs, counted when a chunk of them is read again after a change.
#[derive(Default)]
pub struct Metas {
    pub init: AtomicU64,
    pub inputs: AtomicU64,
    /// `YAVS_CYCLE_STATS=1`: the region's counts, printed with the summary.
    pub(crate) stats: std::sync::Mutex<Option<rt::Stats>>,
    /// The second image of `--cycle-exact-meta` (`rt::Rail`): made, its clock edges, and how it
    /// ended (none while it runs).
    pub(crate) rail: std::sync::atomic::AtomicBool,
    pub(crate) rail_edges: AtomicU64,
    pub(crate) rail_end: std::sync::Mutex<Option<String>>,
}

impl Metas {
    /// The summary line of the metavalues the region read as 0, if it read any.
    pub fn summary(&self) -> Option<String> {
        if let Some(st) = self.stats.lock().expect("not poisoned").take() {
            drop(st);
        }
        let (i, n) = (
            self.init.load(Ordering::Relaxed),
            self.inputs.load(Ordering::Relaxed),
        );
        let mut s = String::new();
        if i + n != 0 {
            s = format!(
                "cycle mode: {} metavalue elements read as 0 ({i} at initialisation, {n} at \
                 changes of the region's inputs)\n",
                i + n
            );
        }
        if self.rail.load(Ordering::Relaxed) {
            let end = self.rail_end.lock().expect("not poisoned").clone();
            let end = end.unwrap_or_else(|| {
                let e = self.rail_edges.load(Ordering::Relaxed);
                format!("still followed when the run ended, after {}", rt::edges(e))
            });
            s += &format!("cycle mode: 'U' start values followed on a second image: {end}\n");
        }
        (!s.is_empty()).then_some(s)
    }
}

/// `file:line` of a span.
fn at(db: &Db, span: yavs_diag::Span) -> String {
    let f = db.sources.get(span.file);
    let file = std::path::Path::new(&f.path)
        .file_name()
        .map_or(f.path.clone(), |n| n.to_string_lossy().into_owned());
    format!("{file}:{}", f.line_col(span.start).0)
}

/// At most this many errors under `--cycle-strict`; one more line counts the rest.
const STRICT_MAX: usize = 40;

/// The error of each reason a process below the top-level unit stays out for (A40): what is not
/// supported (the headline), where (the statement; the other process or signal when it spans
/// processes), the processes, why, and how to fix it. One error per code and statement, its
/// processes listed (a statement of an entity used many times), sorted by file and line.
fn strict_errors(db: &Db, e: &Elaborated, out: &[(usize, region::Why)]) -> Vec<Diagnostic> {
    // By file, line, column and code: one error each, with the text of its first process (the
    // instances of one statement differ in their names only).
    type Key<'a> = (String, u32, u32, &'a str);
    let mut groups: BTreeMap<Key, (&check::Reason, Vec<usize>)> = BTreeMap::new();
    for (pi, why) in out {
        let r = match why {
            region::Why::Subset(r) | region::Why::NotYet(r) => r,
            region::Why::Behavioural(_) => continue,
        };
        if region::in_top_unit(e, *pi) {
            continue;
        }
        let f = db.sources.get(r.span.file);
        let (line, col) = f.line_col(r.span.start);
        let key = (f.path.clone(), line as u32, col as u32, r.code);
        groups.entry(key).or_insert((r, Vec::new())).1.push(*pi);
    }
    let total = groups.len();
    let mut errs: Vec<Diagnostic> = (groups.into_values().take(STRICT_MAX))
        .map(|(r, procs)| reason_error(db, e, r, &procs))
        .collect();
    if total > STRICT_MAX {
        errs.push(Diagnostic::new(
            yavs_diag::Severity::Note,
            "",
            format!(
                "{} more errors not shown; `--cycle-report` lists every process that stays out",
                total - STRICT_MAX
            ),
        ));
    }
    errs
}

/// A reason's text as its construct and detail: split at the first `: ` outside backticks (a
/// quoted name or expression can hold one).
fn construct(text: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '`' => quoted = !quoted,
            ':' if !quoted && text[i + 1..].starts_with(' ') => {
                return (&text[..i], &text[i + 2..]);
            }
            _ => {}
        }
    }
    (text, "")
}

/// One reason as an error: the headline names the construct, the label points at it, the notes
/// say which processes and why, the help how to fix it.
fn reason_error(db: &Db, e: &Elaborated, r: &check::Reason, procs: &[usize]) -> Diagnostic {
    let (_, _, why, fix, limitation) = codes::reason(r.code).copied().unwrap_or((
        r.code,
        "",
        "",
        "run with `--cycle` to leave this process in the event simulation",
        true,
    ));
    let (what, detail) = construct(&r.text);
    let head = if r.code == codes::INTERNAL {
        format!("{what} in cycle mode")
    } else {
        format!("{what} is not supported in cycle mode")
    };
    let mut d = Diagnostic::error(r.code, head).with_primary(r.span, detail);
    if let Some((span, what)) = &r.also {
        d = d.with_secondary(*span, what.clone());
    }
    let names: Vec<String> = procs.iter().map(|&pi| region::where_(db, e, pi)).collect();
    const SHOWN: usize = 5;
    let mut list = names[..names.len().min(SHOWN)].join(", ");
    if names.len() > SHOWN {
        let _ = write!(list, " and {} more", names.len() - SHOWN);
    }
    d = d.with_note(match procs.len() {
        1 => format!("in process {list}"),
        n => format!("in {n} processes: {list}"),
    });
    d = d.with_note(format!("why: {why}"));
    d.with_help(if limitation {
        format!("this is a limitation of yavs, not a problem in your design: {fix}")
    } else {
        format!("{fix}; or run with `--cycle` to leave this process in the event simulation")
    })
}

/// Select the region of `e` and compile it. Under [`Mode::Strict`], a process left out that is
/// neither in the top-level design unit nor behavioural is an error with the code of its
/// reason, all of them listed. Under [`Mode::Report`] only the report is made, nothing is
/// compiled.
pub fn plan(db: &Db, e: &Elaborated, mode: Mode) -> Result<Plan, Vec<Diagnostic>> {
    codegen::skip_check().map_err(|m| vec![Diagnostic::error(codes::ENV_VALUE, m)])?;
    region::exact_clocks().map_err(|m| vec![Diagnostic::error(codes::ENV_VALUE, m)])?;
    rt::rail_knob().map_err(|m| vec![Diagnostic::error(codes::ENV_VALUE, m)])?;
    store::store_bytes().map_err(|m| vec![Diagnostic::error(codes::ENV_VALUE, m)])?;
    rt::skip_knob().map_err(|m| vec![Diagnostic::error(codes::ENV_VALUE, m)])?;
    let sel = match store::select(db, e)? {
        store::Got::Sel(sel) => *sel,
        // `--cycle` on a hit: the region as it was built, its code as it was compiled.
        store::Got::Built(f) if mode == Mode::Run => {
            let (mut b, sites, blob) = *f;
            let (metas, reporter) = (Arc::new(Metas::default()), Reporter::default());
            let code = codegen::from_blob(std::mem::take(&mut b.code), &blob).map_err(|m| {
                vec![
                    Diagnostic::error("E9999", format!("cycle mode cannot load its region: {m}"))
                        .with_note("this is a bug in yavs"),
                ]
            })?;
            return Ok(Plan {
                report: String::new(),
                summary: std::mem::take(&mut b.summary),
                warning: b.warning.take().map(warning),
                region: Some(finish(
                    e,
                    b,
                    rt::Sites::lazy(sites),
                    code,
                    metas.clone(),
                    reporter.clone(),
                )),
                metas,
                reporter,
            });
        }
        store::Got::Built(_) => {
            return Err(vec![
                Diagnostic::error(
                    "E9999",
                    "cycle mode's analysis cache: a built region for a report",
                )
                .with_note("this is a bug in yavs"),
            ]);
        }
    };
    if mode == Mode::Strict {
        let errs = strict_errors(db, e, &sel.out);
        if !errs.is_empty() {
            return Err(errs);
        }
    }
    let total = e.procs.len();
    let head = format!(
        "cycle mode: {} of {total} processes run cycle-based",
        sel.members.len()
    );
    // Inputs nothing drives that start with a metavalue: the normal mode reads that metavalue
    // for the whole run, the region 0. Counted on the summary line: a run that differs this way
    // may never end.
    let undriven: Vec<LeafId> = (region::undriven_metas(e).intersection(&sel.inputs))
        .copied()
        .collect();
    let mut tail = Vec::new();
    if total > sel.members.len() {
        tail.push(format!("{} in the event kernel", total - sel.members.len()));
    }
    match undriven.len() {
        0 => {}
        1 => tail.push("1 input driven by nothing read as 0".to_string()),
        n => tail.push(format!("{n} inputs driven by nothing read as 0")),
    }
    let summary = if tail.is_empty() {
        format!("{head}\n")
    } else {
        format!("{head}, {} (see --cycle-report)\n", tail.join(", "))
    };
    let metas = Arc::new(Metas::default());
    // The region report only for `--cycle-report`: its names and source lines cost a run.
    if mode == Mode::Report || sel.members.is_empty() {
        // Developers (`YAVS_CYCLE_DOWNSTREAM=FILE`): the processes outside that may see what the
        // region exports, one name a line as the console prints it (tools/difftest).
        if mode == Mode::Report
            && let Some(f) = std::env::var_os("YAVS_CYCLE_DOWNSTREAM")
        {
            let text: String = (region::downstream(db, e, &sel).into_iter())
                .map(|pi| region::where_(db, e, pi) + "\n")
                .collect();
            if let Err(x) = std::fs::write(&f, text) {
                eprintln!("YAVS_CYCLE_DOWNSTREAM: {}: {x}", f.to_string_lossy());
            }
        }
        return Ok(Plan {
            report: match mode {
                Mode::Report => report(db, e, &sel, &head, &undriven),
                _ => String::new(),
            },
            summary,
            warning: None,
            region: None,
            metas,
            reporter: Reporter::default(),
        });
    }
    let warning = dropped_warning(db, e, &sel).map(warning);
    let reporter = Reporter::default();
    let region = build(db, e, sel, &summary, metas.clone(), reporter.clone()).map_err(|m| {
        vec![
            Diagnostic::error(
                "E9999",
                format!("cycle mode cannot compile its region: {m}"),
            )
            .with_note("this is a bug in yavs"),
        ]
    })?;
    store::built()?;
    Ok(Plan {
        report: String::new(),
        summary,
        warning,
        region: Some(region),
        metas,
        reporter,
    })
}

/// The asserts and reports cycle-based processes drop, once per statement and process: the
/// severity's name, where, and the process.
fn drops(db: &Db, e: &Elaborated, sel: &region::Selection) -> Vec<(&'static str, String, String)> {
    let mut all: Vec<(u8, bool, String, String)> = (sel.members.iter())
        .flat_map(|(pi, b)| {
            let label = e.hier.proc_label(&e.procs[*pi], yavs_elab::Case::Lrm);
            (b.drops.iter()).map(move |&(span, sev, sure)| (sev, sure, at(db, span), label.clone()))
        })
        .collect();
    // Failure first, then a severity known only at run time (it may be failure), error, warning,
    // note; in each, first those that always report when reached (`assert false`: a design's end).
    all.sort_by_key(|d| {
        let rank = match d.0 {
            3 => 0,
            lower::SEV_RUNTIME => 1,
            s => 4 - s,
        };
        (rank, !d.1)
    });
    (all.into_iter())
        .map(|(sev, _, at, who)| {
            let name = ["note", "warning", "error", "failure"]
                .get(sev as usize)
                .copied()
                .unwrap_or("run-time");
            (name, at, who)
        })
        .collect()
}

/// W4101 (A41) from its headline and notes ([`dropped_warning`], kept so in `Built`).
fn warning((head, notes): (String, Vec<String>)) -> Diagnostic {
    let d = Diagnostic::warning(codes::DROPPED_STOPS, head);
    (notes.into_iter().fold(d, Diagnostic::with_note)).with_help(
        "these do not stop the simulation in cycle mode; run without `--cycle` to evaluate them, \
         or with `--cycle-report` instead to list every assert and report cycle mode drops",
    )
}

/// The headline and notes of W4101 (A41): the asserts and reports of severity error or failure
/// (or one known only at run time) that cycle-based processes drop, the first few with where
/// they are.
fn dropped_warning(
    db: &Db,
    e: &Elaborated,
    sel: &region::Selection,
) -> Option<(String, Vec<String>)> {
    const SHOWN: usize = 5;
    let stops: Vec<_> = (drops(db, e, sel).into_iter())
        .filter(|d| matches!(d.0, "failure" | "error" | "run-time"))
        .collect();
    if stops.is_empty() {
        return None;
    }
    let sev = |s: &str| match s {
        "run-time" => "a severity known only at run time".to_string(),
        s => format!("severity {s}"),
    };
    let n = stops.len();
    let head = if n == 1 {
        format!(
            "1 assert or report of {} is not evaluated in cycle mode",
            sev(stops[0].0)
        )
    } else {
        let by: Vec<String> = ["failure", "error", "run-time"]
            .iter()
            .map(|s| (stops.iter().filter(|d| d.0 == *s).count(), *s))
            .filter(|c| c.0 > 0)
            .map(|(c, s)| {
                format!(
                    "{c} {}",
                    if s == "run-time" {
                        "known at run time"
                    } else {
                        s
                    }
                )
            })
            .collect();
        format!(
            "{n} asserts and reports of severity error or failure are not evaluated in cycle mode ({})",
            by.join(", ")
        )
    };
    let mut notes = Vec::new();
    for (s, at, who) in stops.iter().take(SHOWN) {
        notes.push(format!("{at}: {}, in {who}", sev(s)));
    }
    if n > SHOWN {
        notes.push(format!("and {} more", n - SHOWN));
    }
    Some((head, notes))
}

/// The region report of `sel` (`--cycle-report`): what joined and what stays in the event kernel,
/// with reasons; `head` its first line, `undriven` the inputs nothing drives that start with a
/// metavalue.
fn report(
    db: &Db,
    e: &Elaborated,
    sel: &region::Selection,
    head: &str,
    undriven: &[LeafId],
) -> String {
    let label = |pi: usize| e.hier.proc_label(&e.procs[pi], yavs_elab::Case::Lrm);
    let undriven: Vec<String> = undriven.iter().map(|&l| e.leaf_name(l)).collect();
    let total = e.procs.len();
    let mut report = String::new();
    report.push_str(head);
    let edges: Vec<String> = (sel.clocks.iter())
        .map(|c| {
            format!(
                "the {} edge of {}{}",
                if c.rising { "rising" } else { "falling" },
                region::clock_name(e, c),
                if c.legacy { " (`'event` form)" } else { "" }
            )
        })
        .collect();
    if !edges.is_empty() {
        let _ = write!(report, ", on {}", edges.join(" and "));
    }
    report.push('\n');
    // The processes left out, grouped by reason (its code; behavioural processes and the ones
    // not implemented yet are one group each), the most common first. Rows: where, what.
    let mut left: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (pi, why) in &sel.out {
        let (head, text) = match why {
            region::Why::Subset(r) | region::Why::NotYet(r) => (
                format!("{} {}", r.code, title(r.code)),
                format!("{} {} ({})", r.code, r.text, at(db, r.span)),
            ),
            region::Why::Behavioural(w) => {
                ("behavioural, in the event kernel".to_string(), w.clone())
            }
        };
        left.entry(head)
            .or_default()
            .push((region::where_(db, e, *pi), text));
    }
    let mut left: Vec<_> = left.into_iter().collect();
    left.sort_by_key(|g| std::cmp::Reverse(g.1.len()));
    let members: Vec<(String, String)> = (sel.members.iter())
        .map(|(pi, b)| {
            let k = match b.kind {
                Kind::Clocked => "clocked",
                Kind::Comb => "combinational",
            };
            let mut k = k.to_string();
            match b.metas {
                0 => {}
                1 => k.push_str(", 1 literal with a metavalue read as 0"),
                n => {
                    let _ = write!(k, ", {n} literals with a metavalue read as 0");
                }
            }
            (region::where_(db, e, *pi), k)
        })
        .collect();
    let w = (left.iter().flat_map(|g| &g.1).chain(&members))
        .map(|r| r.0.chars().count())
        .max()
        .unwrap_or(0)
        .min(60); // one long name does not pad every row
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    if !sel.out.is_empty() {
        let _ = writeln!(
            report,
            "  {} stay in the event kernel:",
            plural(sel.out.len(), "process", "processes")
        );
    }
    for (head, rows) in &left {
        let _ = writeln!(
            report,
            "    {head} ({}):",
            plural(rows.len(), "process", "processes")
        );
        for (name, what) in rows {
            let _ = writeln!(report, "      {name:<w$}  {what}");
        }
    }
    if !undriven.is_empty() {
        let _ = writeln!(
            report,
            "  {} the region reads without a driver, each keeping its metavalue start value for \
             the whole run (the region reads 0):",
            plural(undriven.len(), "signal", "signals")
        );
        for n in &undriven {
            let _ = writeln!(report, "    {n}");
        }
    }
    // Asserts and reports in members are compiled to nothing.
    let held: Vec<(String, u32)> = (sel.members.iter())
        .filter(|(_, b)| b.dropped > 0)
        .map(|(pi, b)| (region::where_(db, e, *pi), b.dropped))
        .collect();
    if !held.is_empty() {
        let _ = writeln!(
            report,
            "  {} and reports not evaluated in cycle mode, held by {}:",
            plural(held.iter().map(|h| h.1 as usize).sum(), "assert", "asserts"),
            plural(held.len(), "cycle-based process", "cycle-based processes")
        );
        for (name, n) in &held {
            let _ = writeln!(report, "    {n:>4}  {name}");
        }
        let all = drops(db, e, sel);
        let mut by: Vec<String> = Vec::new();
        for s in ["failure", "run-time", "error", "warning", "note"] {
            match all.iter().filter(|d| d.0 == s).count() {
                0 => {}
                n => by.push(format!(
                    "{n} {}",
                    if s == "run-time" {
                        "known at run time"
                    } else {
                        s
                    }
                )),
            }
        }
        let _ = writeln!(
            report,
            "  each assert and report not evaluated, by severity ({}):",
            by.join(", ")
        );
        let w = all.iter().map(|d| d.1.chars().count()).max().unwrap_or(0);
        for (sev, at, who) in &all {
            let _ = writeln!(report, "    {sev:<8}  {at:<w$}  {who}");
        }
    }
    if !sel.members.is_empty() {
        report.push_str(
            "  cycle-based processes give no warnings of the ieee libraries (numeric_std \
             metavalue and compare warnings)\n",
        );
        report.push_str(
            "  asserts and reports of processes of the top-level unit are evaluated: at the \
             clock edge in region code, else the process stays in the event kernel\n",
        );
    }
    let _ = writeln!(
        report,
        "  subset: {} of {total} processes fit, {} loop group{} among them",
        sel.fits,
        sel.groups.len(),
        if sel.groups.len() == 1 { "" } else { "s" }
    );
    for g in &sel.groups {
        let names: Vec<String> = g.procs.iter().take(3).map(|&pi| label(pi)).collect();
        let more = match g.procs.len() {
            n if n > 3 => format!(" and {} more", n - 3),
            _ => String::new(),
        };
        let _ = writeln!(
            report,
            "    loop group of {} process{}: {} passes, or {} pieces after a split: {}{more}",
            g.procs.len(),
            if g.procs.len() == 1 { "" } else { "es" },
            g.k,
            g.slices,
            names.join(", ")
        );
    }
    for l in &sel.loops {
        let _ = writeln!(
            report,
            "    in the region: a loop group of {} process{}, {} process runs per settle ({} passes \
             of all would be {})",
            l.procs,
            if l.procs == 1 { "" } else { "es" },
            l.at.len(),
            l.k,
            l.procs * l.k as usize
        );
    }
    if sel.members.is_empty() {
        return report;
    }
    let name = |l: &LeafId| e.leaf_name(*l);
    let _ = writeln!(
        report,
        "  boundary: {} signal leaves exported ({} nothing outside waits on), {} inputs, {} wake \
         the region{}",
        sel.outputs.len(),
        sel.outputs.difference(&sel.waited).count(),
        sel.inputs.len(),
        sel.wakes.len(),
        match sel.wakes.iter().map(name).collect::<Vec<_>>() {
            v if v.is_empty() => String::new(),
            v => format!(" ({})", v.join(", ")),
        }
    );
    // Which wakes are settled at once, and why; the others wait for the next edge.
    if !sel.wakes.is_empty() {
        let why = |r: &region::Reach| match *r {
            region::Reach::Output(o, Some(p)) => format!(
                "{} read by {}",
                e.leaf_name(o),
                e.hier.proc_label(&e.procs[p], yavs_elab::Case::Lrm)
            ),
            region::Reach::Output(o, None) => format!("{} read outside", e.leaf_name(o)),
            region::Reach::Check(p) => format!(
                "a check in {}",
                e.hier.proc_label(&e.procs[p], yavs_elab::Case::Lrm)
            ),
        };
        let at_once: Vec<String> = (sel.at_once.iter())
            .map(|(l, r)| format!("{} to {}", name(l), why(r)))
            .collect();
        let _ = writeln!(
            report,
            "  wakes: {} settled at once, through combinational code to what outside sees{}; {} \
             read when they change and settled at the next edge",
            at_once.len(),
            if at_once.is_empty() {
                String::new()
            } else {
                format!(" ({})", at_once.join(", "))
            },
            sel.wakes.len() - at_once.len()
        );
    }
    let _ = writeln!(
        report,
        "  cycle-based, {}:",
        plural(members.len(), "process", "processes")
    );
    for (name, what) in &members {
        let _ = writeln!(report, "    {name:<w$}  {what}");
    }
    report
}

/// Lay out the image, schedule and compile the region, and make its kernel process; keep what
/// was built with its machine code when the selection is cached (`store::keep_built`).
fn build(
    db: &Db,
    e: &Elaborated,
    sel: region::Selection,
    summary: &str,
    metas: Arc<Metas>,
    reporter: Reporter,
) -> Result<Region, String> {
    let (mut b, code, made) = built(db, e, &sel, summary)?;
    drop(sel);
    store::keep_built(&b, made)?;
    let sites = rt::Sites::new(std::mem::take(&mut b.checks));
    Ok(finish(e, b, sites, code, metas, reporter))
}

/// Lay out the image, schedule and compile the region: what its kernel process is made from,
/// its code, and the machine code this compile made (for the cache).
fn built(
    db: &Db,
    e: &Elaborated,
    sel: &region::Selection,
    summary: &str,
) -> Result<(Built, codegen::Code, Option<store::Blob>), String> {
    let nets = &e.nets;
    let ksig = |l: LeafId| nets.loc[l.index()].signal.0;
    // A kernel signal of logic elements: its length; a byte signal (integers or enumerations): a
    // word per element.
    let bytes = |k: u32| match nets.leaf(nets.packs[k as usize].0).ty.elem {
        yavs_kernel::Elem::Bytes(b) => b,
        yavs_kernel::Elem::Logic => 0,
    };
    let logic = |k: u32| bytes(k) == 0;
    let klen = |k: u32| {
        let n = nets.signal_len(yavs_kernel::SignalId(k));
        if logic(k) { n } else { 64 * n }
    };
    // Memories clocked members write: their writes wait for the commit (`defer`), so they need no
    // next-state block (`region::ram_rule` kept only writes at run-time indices).
    let mut rams: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for (_, b) in sel.members.iter().filter(|(_, b)| b.kind == Kind::Clocked) {
        let mut w = BTreeMap::new();
        region::sig_writes(&b.ops, &mut w);
        rams.extend(
            (w.into_iter())
                .filter(|&(k, h)| h.indexed && region::big_signal(e, k))
                .map(|(k, _)| k),
        );
    }
    let several = sel.clocks.len() > 1;
    // Per next-state signal, with several classes: the classes whose clocked members write it
    // (`u32::MAX`: a member without one, every class). Its copies run after an edge of one of
    // them only (twice when two had one).
    let named = named_sigs(sel);
    let mut writers: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (k, (_, b)) in sel.members.iter().enumerate() {
        if !several || b.kind != Kind::Clocked {
            continue;
        }
        let c = (b.clock)
            .and_then(|c| sel.clocks.iter().position(|&x| x == c))
            .map_or(u32::MAX, |c| c as u32);
        for s in b
            .writes
            .iter()
            .map(|&l| ksig(l))
            .chain(named[k][1].iter().copied())
        {
            writers.entry(s).or_default().insert(c);
        }
    }
    // The next-state signals by the classes that write them: their current words lie first in
    // this order, and their next-state words in the same, so the copies of one set of classes are
    // one run each way (`codegen::Step::Copy`).
    let mut nexts: Vec<(Vec<u32>, u32)> = (sel.next.iter())
        .map(|&l| ksig(l))
        .filter(|k| !rams.contains(k))
        .map(|k| {
            let ws = writers.get(&k).into_iter().flatten().copied().collect();
            (ws, k)
        })
        .collect();
    nexts.sort_unstable();
    nexts.dedup();
    // Image: the words of each kernel signal of the region (its leaves at their kernel offsets),
    // then next-state words, then the slots of each member, then two words for a failed check.
    let mut cur: BTreeMap<u32, u32> = BTreeMap::new();
    let mut n = 0u32;
    let leaves = (sel.driven.iter().chain(&sel.inputs)).chain(
        sel.members
            .iter()
            .flat_map(|(_, b)| b.reads.iter().chain(&b.writes)),
    );
    for k in nexts.iter().map(|x| x.1).chain(leaves.map(|&l| ksig(l))) {
        cur.entry(k).or_insert_with(|| {
            n += codegen::words(klen(k));
            n - codegen::words(klen(k))
        });
    }
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for &(_, k) in &nexts {
        next.entry(k).or_insert_with(|| {
            n += codegen::words(klen(k));
            n - codegen::words(klen(k))
        });
    }
    // Signals hold state between calls; so do the variables of clocked processes.
    let signal_words = n;
    // One word per edge class: 1 when its edge happened in this wake (`Ref::Edge` of its
    // members).
    let edge_words: Vec<u32> = (0..sel.clocks.len() as u32).map(|k| n + k).collect();
    n += sel.clocks.len().max(1) as u32;
    // Under `--cycle-exact-meta` the two words of `Ref::Oth` (`rt::Rail`).
    let oth = n;
    if region::exact_meta() {
        n += 2;
    }
    let mut state: Vec<(u32, u32)> = Vec::new();
    // Each function as pieces, a member's body each (`codegen::compile` cuts between them).
    // Each step with the member it runs (`codegen::compile`'s units; `u32::MAX`: none).
    let mut settle: Vec<(codegen::Step, u32)> = Vec::new();
    let mut edge: Vec<codegen::Step> = Vec::new();
    let mut init_fn: Vec<codegen::Step> = Vec::new();
    // The edge words of a set of classes.
    let class_words = |ws: &[u32]| -> Vec<u32> {
        if ws.contains(&u32::MAX) {
            edge_words.clone()
        } else {
            ws.iter().map(|&c| edge_words[c as usize]).collect()
        }
    };
    // The copies into the next-state blocks (`into`) or back, behind one gate per set of classes
    // whose members write them (`codegen::Step::Gate`: any of them had an edge); adjacent words
    // join into one run.
    let copies = |into: bool| -> Vec<codegen::Step> {
        let mut by: BTreeMap<Vec<u32>, Vec<(u32, u32, u32)>> = BTreeMap::new();
        for (ws, k) in &nexts {
            let (dst, src) = if into {
                (next[k], cur[k])
            } else {
                (cur[k], next[k])
            };
            let len = codegen::words(klen(*k));
            let runs = by.entry(class_words(ws)).or_default();
            match runs.last_mut() {
                Some(r) if r.0 + r.2 == dst && r.1 + r.2 == src => r.2 += len,
                _ => runs.push((dst, src, len)),
            }
        }
        (by.into_iter())
            .flat_map(|(w, runs)| [codegen::Step::Gate(w), codegen::Step::Copy(runs)])
            .collect()
    };
    edge.extend(copies(true));
    // The edge word of each member's class (none: a clocked member without one, behind a test of
    // its own).
    let class_word = |k: usize| -> Option<u32> {
        (sel.members[k].1.clock)
            .and_then(|c| sel.clocks.iter().position(|&x| x == c))
            .map(|c| edge_words[c])
    };
    let mut init: Vec<(u32, u64)> = Vec::new();
    let mut checks: Vec<(lower::Check, Vec<yavs_diag::Span>, usize)> = Vec::new();
    let mut uninit = 0u64;
    let mut useed: Vec<(u32, u32)> = Vec::new();
    // The bits clocked members keep between edges (word, bits): their signals' elements, their
    // variables and their memory writes' words; only of members whose comparisons the second
    // image may decide otherwise than the normal mode (`Body::keep`).
    let mut regs: BTreeMap<u32, u64> = BTreeMap::new();
    let reg_words = |regs: &mut BTreeMap<u32, u64>, w: u32, c: u32| {
        (w..w + c).for_each(|i| *regs.entry(i).or_default() = !0);
    };
    // Each member's slots and resolution.
    let none = BTreeMap::new();
    let mut rs: Vec<Resolve> = Vec::new();
    // Per member: its clocked body with the memory writes kept back, those writes, and the slot
    // that says one of them happened.
    let mut deferred: Vec<(Vec<Op>, Vec<Pending>, u32)> = Vec::new();
    let skip_on = rt::skip_knob()?;
    let mut track_at: Vec<Option<u32>> = Vec::new();
    for (pi, b) in &sel.members {
        let base = n;
        n += b.locs;
        let mut pend = Vec::new();
        let any = b.locs;
        let ops = if b.kind == Kind::Clocked && !rams.is_empty() {
            let mut at = any + 1;
            let ops = defer(&b.ops, &rams, any, &mut at, &mut pend);
            if pend.is_empty() {
                at = b.locs;
            }
            // The writes' words hold state from the body to the commit.
            state.push((base + b.locs, at - b.locs));
            if b.keep {
                reg_words(&mut regs, base + b.locs, at - b.locs);
            }
            n += at - b.locs;
            ops
        } else {
            Vec::new()
        };
        deferred.push((ops, pend, any));
        // A clocked member's flag (`Op::Track`) and run word (`rt::Skip`), after its other slots.
        track_at.push((b.kind == Kind::Clocked && skip_on).then(|| {
            n += 2;
            n - 2 - base
        }));
        init.extend(b.init.iter().map(|&(w, v)| (base + w, v)));
        let sites = checks.len() as u32;
        checks.extend(b.checks.iter().map(|(c, at)| {
            let c = match c {
                // A note's words, at the member's place in the image.
                lower::Check::Note(nt) => {
                    let mut nt = nt.clone();
                    for a in &mut nt.args {
                        if let Ref::Loc(w) = a.0 {
                            a.0 = Ref::Loc(base + w);
                        }
                    }
                    lower::Check::Note(nt)
                }
                c => c.clone(),
            };
            (c, at.clone(), *pi)
        }));
        uninit += b.uninit;
        let clocked = b.kind == Kind::Clocked;
        if clocked {
            state.extend(b.vars.iter().map(|&(w, c)| (base + w, c)));
        }
        // The second image's tables, only where it can run (the default keeps none).
        if clocked && region::exact_meta() {
            useed.extend(b.useed.iter().map(|&(w, c)| (base + w, c)));
        }
        if clocked && region::exact_meta() && b.keep {
            for &(w, c) in &b.vars {
                reg_words(&mut regs, base + w, c);
            }
            for &l in &b.writes {
                let (k, at, len) = (ksig(l), nets.loc[l.index()].base, nets.leaf(l).ty.len);
                if !logic(k) {
                    reg_words(&mut regs, cur[&k], codegen::words(klen(k)));
                    continue;
                }
                for i in at..at + len {
                    *regs.entry(cur[&k] + i / 64).or_default() |= 1 << (i % 64);
                }
            }
        }
        let class = b
            .clock
            .and_then(|c| sel.clocks.iter().position(|&k| k == c));
        rs.push(Resolve {
            base,
            sites,
            clocked,
            edge: class.map_or(signal_words, |k| edge_words[k]),
            oth,
            rail: true,
            cur: &cur,
            next: &next,
            snap: &none,
        });
    }
    // A clocked member's edge body: its stores tracked (`Op::Track`) and, unless it logs notes or
    // tests the edges of several classes itself, behind its run word (`rt::Skip`).
    let skips = |k: usize| sel.members[k].1.notes == 0 && (!several || class_word(k).is_some());
    let tracked = |k: usize, body: Vec<Op>| -> Vec<Op> {
        let Some(t) = track_at[k] else {
            return body;
        };
        let mut v = vec![Op::Track {
            flag: Ref::Loc(t),
            first: true,
        }];
        if skips(k) {
            v.push(Op::If {
                cond: Ref::Loc(t + 1),
                then: body,
                else_: Vec::new(),
            });
        } else {
            v.extend(body);
        }
        v
    };
    // The settle: the combinational members and, while their reset holds, the reset branches
    // of clocked ones (written to the current words: the commit is past). A member of a loop group
    // is a body of its own, run at each pass level of what it drives: checked at its last run, on
    // settled values, and without checks before.
    let looped: std::collections::BTreeSet<usize> = (sel.loops.iter())
        .flat_map(|l| sel.settle[l.at.clone()].iter().copied())
        .collect();
    let piece = |k: usize, snap: &BTreeMap<u32, u32>| -> Result<Vec<Op>, String> {
        let (_, b) = &sel.members[k];
        let mut ops: Vec<Op> = (snap.iter())
            .map(|(&s, &w)| Op::Copy {
                dst: Ref::Loc(w),
                src: Ref::Loc(cur[&s]),
                len: klen(s),
            })
            .collect();
        // A loop group's member reads values of earlier passes: no `Op::Other` there.
        let r = Resolve {
            clocked: false,
            snap,
            rail: !looped.contains(&k),
            ..rs[k]
        };
        if b.kind == Kind::Clocked {
            let piece = region::reset_piece(&b.ops).ok_or("a reset branch of another shape")?;
            ops.extend(piece.iter().map(|op| r.op(op)));
        } else {
            ops.extend(b.ops.iter().map(|op| r.op(op)));
        }
        Ok(ops)
    };
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    if stats {
        shapes(db, e, sel, &next);
    }
    // One body per shape: members of `SHARE_MIN` operations or more, not in a loop group, whose
    // code is the same but for where their words lie; a combinational member's settle body, a
    // clocked member's edge body.
    let mut by_key: std::collections::HashMap<ShapeKey, Vec<usize>, codegen::KeyBuild> =
        Default::default();
    for (k, (_, b)) in sel.members.iter().enumerate() {
        // A member with memory writes kept back for the commit keeps its own code.
        if !looped.contains(&k)
            && deferred[k].1.is_empty()
            && (b.kind != Kind::Clocked || !several || class_word(k).is_some())
            && b.notes == 0
            && b.ops.iter().map(codegen::weight_of).sum::<usize>() >= SHARE_MIN
        {
            by_key.entry(shape_key(b, &next)).or_default().push(k);
        }
    }
    // In the order of their first members, so the code is the same at every run.
    let mut by_key: Vec<Vec<usize>> = by_key.into_values().collect();
    by_key.sort_unstable();
    // Per member that shares: its shape and its blocks.
    let mut shared: BTreeMap<usize, (usize, Blocks)> = BTreeMap::new();
    // Per shape: its leader's body and blocks.
    let mut leaders: Vec<(Vec<Op>, Blocks, Resolve)> = Vec::new();
    for ks in by_key.into_iter().filter(|ks| ks.len() > 1) {
        let body = |k: usize| {
            let b = &sel.members[k].1;
            let ops = shared_body(
                &b.ops,
                b.kind == Kind::Clocked,
                several && class_word(k).is_none(),
            );
            if b.kind == Kind::Clocked {
                tracked(k, ops)
            } else {
                ops
            }
        };
        let extra = |k: usize| if track_at[k].is_some() { 2 } else { 0 };
        let lead = blocks(
            &body(ks[0]),
            &sel.members[ks[0]].1,
            &rs[ks[0]],
            klen,
            extra(ks[0]),
        );
        let mut members = Vec::new();
        for &k in &ks {
            let bl = blocks(&body(k), &sel.members[k].1, &rs[k], klen, extra(k));
            // The same blocks in the same order with the same lengths, or the member keeps
            // its own code.
            if bl.len() == lead.len()
                && (bl.iter().zip(&lead)).all(|(x, y)| same_kind(x.0, y.0) && x.2 == y.2)
            {
                members.push((k, bl));
            }
        }
        if members.len() > 1 {
            let sh = leaders.len();
            leaders.push((body(members[0].0), lead, rs[members[0].0]));
            shared.extend(members.into_iter().map(|(k, bl)| (k, (sh, bl))));
        }
    }
    if stats {
        eprintln!(
            "cycle stats: {} members share {} bodies",
            shared.len(),
            leaders.len()
        );
        let clocked: Vec<usize> = (0..leaders.len())
            .filter(|&k| leaders[k].2.clocked)
            .collect();
        eprintln!("cycle stats: bodies of clocked members: {clocked:?}");
    }
    let call = |k: usize| -> codegen::Step {
        let (shape, bl) = &shared[&k];
        codegen::Step::Shared {
            shape: *shape,
            table: bl.iter().map(|b| b.1).collect(),
            sites: rs[k].sites,
        }
    };
    let weight = |ops: &[Op]| ops.iter().map(codegen::weight_of).sum::<usize>();
    // The image words of signal leaves, as (first, count); for the inputs of `b` (`Some`) also
    // the whole words of a signal its operations read but none of whose leaves it lists.
    let leaf_words = |ls: &BTreeSet<LeafId>, b: Option<&lower::Body>| -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = Vec::new();
        for &l in ls {
            let (s, len) = (ksig(l), nets.leaf(l).ty.len);
            let Some(&slot) = cur.get(&s).filter(|_| len > 0) else {
                continue;
            };
            let base = nets.loc[l.index()].base;
            v.push(if logic(s) {
                (slot + base / 64, (base + len - 1) / 64 - base / 64 + 1)
            } else {
                (slot, codegen::words(klen(s)))
            });
        }
        if let Some(b) = b {
            let listed: BTreeSet<u32> = b.reads.iter().chain(&b.writes).map(|&l| ksig(l)).collect();
            let srcs = std::cell::RefCell::new(BTreeSet::new());
            for op in &b.ops {
                op.map(
                    &|r| {
                        if let Ref::Sig(k) = r {
                            srcs.borrow_mut().insert(k);
                        }
                        r
                    },
                    &|r| r,
                    &|x| x,
                );
            }
            for k in srcs.into_inner().difference(&listed) {
                if let Some(&slot) = cur.get(k) {
                    v.push((slot, codegen::words(klen(*k))));
                }
            }
        }
        v
    };
    // Sorted and joined; spans of inputs a few words apart are one (a compare of a span costs more
    // than of a word).
    let join = |mut v: Vec<(u32, u32)>, gap: u32| -> Vec<(u32, u32)> {
        v.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::new();
        for (w, c) in v {
            match out.last_mut() {
                Some(x) if w <= x.0 + x.1 + gap => x.1 = x.1.max(w + c - x.0),
                _ => out.push((w, c)),
            }
        }
        out
    };
    // A settle step outside loop groups: a shared body, or the member's own operations.
    let plain_step = |k: usize,
                      settle: &mut Vec<(codegen::Step, u32)>,
                      guard: &mut dyn FnMut(usize, usize) -> Option<usize>|
     -> Result<(), String> {
        let b = &sel.members[k].1;
        let step = if b.kind != Kind::Clocked && shared.contains_key(&k) {
            (call(k), weight(&b.ops))
        } else {
            let ops = piece(k, &none)?;
            let w = weight(&ops);
            (codegen::Step::Ops(ops), w)
        };
        if let Some(g) = guard(k, step.1) {
            settle.push((codegen::Step::Guard(g, true, 1), u32::MAX));
        }
        settle.push((step.0, k as u32));
        Ok(())
    };
    let confirm = codegen::confirm_on();
    // Guards (`codegen::Guard`): combinational members heavy enough that comparing the words
    // they read with those of their last run costs little against running them. Developers:
    // `YAVS_CYCLE_GUARD=all` guards every settle step, reset branches included (for the tests).
    let guard_all = std::env::var_os("YAVS_CYCLE_GUARD").is_some_and(|v| v == "all");
    let mut guard_of: BTreeMap<usize, usize> = BTreeMap::new();
    let guards: std::cell::RefCell<Vec<GuardSpec>> = Default::default();
    let label = |k: usize| {
        e.hier
            .proc_label(&e.procs[sel.members[k].0], yavs_elab::Case::Lrm)
    };
    let mut guard = |k: usize, weight: usize| -> Option<usize> {
        if let Some(&g) = guard_of.get(&k) {
            return Some(g);
        }
        let b = &sel.members[k].1;
        if !guard_all && (b.kind == Kind::Clocked || weight < GUARD_MIN) {
            return None;
        }
        let ins = join(leaf_words(&b.reads, Some(b)), GUARD_GAP);
        if !guard_all && ins.iter().map(|x| x.1 as usize).sum::<usize>() > weight {
            return None;
        }
        let mut gs = guards.borrow_mut();
        gs.push((label(k), ins, join(leaf_words(&b.writes, None), 0)));
        guard_of.insert(k, gs.len() - 1);
        Some(gs.len() - 1)
    };
    let mut bodies: Vec<(Vec<Op>, bool)> = Vec::new();
    let mut groups: Vec<(Vec<(u32, u32)>, u32)> = Vec::new();
    let mut at = 0;
    for l in &sel.loops {
        for &k in &sel.settle[at..l.at.start] {
            plain_step(k, &mut settle, &mut guard)?;
        }
        at = l.at.end;
        let steps = &sel.settle[l.at.clone()];
        // Per member: its bodies (checked, and unchecked when it runs more than once).
        let mut body: BTreeMap<usize, (usize, Option<usize>)> = BTreeMap::new();
        let mut ks: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for &k in steps {
            if body.contains_key(&k) {
                continue;
            }
            let b = &sel.members[k].1;
            ks.extend(b.writes.iter().map(|&w| ksig(w)));
            // It may read what it drives: from a copy made at its start, the values from before
            // it ran, as a process reads them (its writes are in place).
            let mut snap: BTreeMap<u32, u32> = BTreeMap::new();
            for &l in b.reads.intersection(&b.writes) {
                snap.entry(ksig(l)).or_insert_with(|| {
                    n += codegen::words(klen(ksig(l)));
                    n - codegen::words(klen(ksig(l)))
                });
            }
            let ops = piece(k, &snap)?;
            let early = (steps.iter().filter(|&&s| s == k).count() > 1).then(|| {
                bodies.push((ops.clone(), false));
                bodies.len() - 1
            });
            bodies.push((ops, true));
            body.insert(k, (bodies.len() - 1, early));
        }
        // The whole group behind one guard too, on what its members read that none of them
        // drives: the group's values are a function of those alone (its element graph has no
        // loop), so they are unchanged when those are.
        // Driven by the group alone: leaves whose every driver is a combinational member of it (a
        // reset branch's register also changes at the edge, another process may drive other
        // elements of a leaf).
        let (mut reads, mut writes) = (BTreeSet::new(), BTreeSet::new());
        let mut comb: BTreeSet<usize> = BTreeSet::new();
        for &k in body.keys() {
            let b = &sel.members[k].1;
            reads.extend(b.reads.iter().copied());
            writes.extend(b.writes.iter().copied());
            if b.kind == Kind::Comb {
                comb.insert(sel.members[k].0);
            }
        }
        let mut inside = writes.clone();
        for d in nets.drivers.iter().filter(|d| writes.contains(&d.leaf)) {
            if !comb.contains(&d.proc.index()) {
                inside.remove(&d.leaf);
            }
        }
        let mut ins = leaf_words(&reads.difference(&inside).copied().collect(), None);
        for &k in body.keys() {
            let b = &sel.members[k].1;
            ins.extend(leaf_words(&BTreeSet::new(), Some(b)));
        }
        let g = {
            let mut gs = guards.borrow_mut();
            gs.push((
                format!(
                    "the loop group of {}",
                    label(*body.keys().next().expect("a member"))
                ),
                join(ins, GUARD_GAP),
                join(leaf_words(&writes, None), 0),
            ));
            gs.len() - 1
        };
        settle.push((codegen::Step::Guard(g, true, steps.len()), u32::MAX));
        for (i, &k) in steps.iter().enumerate() {
            let (checked, early) = body[&k];
            let last = !steps[i + 1..].contains(&k);
            if let Some(g) = guard(k, weight(&bodies[checked].0)) {
                settle.push((codegen::Step::Guard(g, last, 1), u32::MAX));
            }
            let b = if last {
                checked
            } else {
                early.expect("a member run twice has a body without checks")
            };
            settle.push((codegen::Step::Call(b), k as u32));
        }
        // Debug builds: one more pass changes nothing the group drives.
        let words: Vec<(u32, u32)> = (ks.into_iter())
            .map(|k| (cur[&k], codegen::words(klen(k))))
            .collect();
        if confirm {
            let g = groups.len();
            settle.push((codegen::Step::Save(g), u32::MAX));
            settle.extend(body.values().map(|b| (codegen::Step::Call(b.0), u32::MAX)));
            settle.push((codegen::Step::Same(g), u32::MAX));
        }
        groups.push((words, 0));
    }
    for &k in &sel.settle[at..] {
        plain_step(k, &mut settle, &mut guard)?;
    }
    // The edge: the clocked bodies of the classes whose edge happened, each class behind one gate
    // (with one class only at its edge, so behind none), the commit; then the settle's functions.
    // By class, in member order within one; in member order alone when members of two classes log
    // notes (their order in a delta where both have an edge).
    for (k, (_, b)) in sel.members.iter().enumerate() {
        let r = &rs[k];
        init_fn.push(codegen::Step::Ops(
            b.init_ops.iter().map(|op| r.op(op)).collect(),
        ));
    }
    let gate_of =
        |k: usize| -> Vec<u32> { class_word(k).filter(|_| several).into_iter().collect() };
    let mut clocked: Vec<usize> = (0..sel.members.len())
        .filter(|&k| sel.members[k].1.kind == Kind::Clocked)
        .collect();
    let noted: BTreeSet<Vec<u32>> = (clocked.iter())
        .filter(|&&k| sel.members[k].1.notes > 0)
        .map(|&k| gate_of(k))
        .collect();
    if noted.len() < 2 {
        clocked.sort_by_key(|&k| gate_of(k));
    }
    let mut gate: Vec<u32> = Vec::new();
    for k in clocked {
        let (r, b) = (&rs[k], &sel.members[k].1);
        let w = gate_of(k);
        if w != gate {
            edge.push(codegen::Step::Gate(w.clone()));
            gate = w;
        }
        if shared.contains_key(&k) {
            edge.push(call(k));
        } else {
            // A body with memory writes kept back has them in `deferred`.
            let ops = if deferred[k].1.is_empty() {
                &b.ops
            } else {
                &deferred[k].0
            };
            let body = tracked(
                k,
                shared_body(ops, true, several && class_word(k).is_none()),
            );
            let ops = body.iter().map(|op| r.op(op)).collect();
            edge.push(codegen::Step::Ops(ops));
        }
    }
    // The memory writes of this edge, in member and program order, then the next-state blocks.
    // A member's are behind the word its body sets with each one's flag.
    // A memory something outside reads keeps the flags of its writes for the export, which
    // drives only what they changed and clears them (`rt::RamWrite`).
    let exported = |k: u32| {
        let (a, z) = nets.packs[k as usize];
        (a.0..z.0).any(|l| sel.outputs.contains(&LeafId(l)))
    };
    // By the set of classes whose edge runs them (a member's flags are set only at its edge),
    // each in member and program order: an edge of another class skips them.
    let mut writes: BTreeMap<Vec<u32>, Vec<Op>> = BTreeMap::new();
    let mut ram_writes: BTreeMap<u32, Vec<rt::RamWrite>> = BTreeMap::new();
    for (k, (_, pend, any)) in deferred.iter().enumerate() {
        if pend.is_empty() {
            continue;
        }
        let r = &rs[k];
        let gate = match class_word(k) {
            _ if !several => Vec::new(),
            Some(w) => vec![w],
            None => edge_words.clone(),
        };
        // Behind the member's word that one of them happened, cleared after them.
        let mut ifs = Vec::new();
        for p in pend {
            let Op::IndexWrite {
                dst: Ref::Sig(sig),
                idx: Ref::Loc(i),
                lo,
                n,
                stride,
                w,
                left,
                down,
                ..
            } = p.write
            else {
                return Err("cycle mode: a kept-back memory write of another shape".into());
            };
            let keep = exported(sig);
            if keep {
                ram_writes.entry(sig).or_default().push(rt::RamWrite {
                    flag: (r.base + p.flag) as usize,
                    idx: (r.base + i) as usize,
                    lo,
                    n,
                    stride,
                    w,
                    left,
                    down,
                    class: (sel.members[k].1.clock)
                        .and_then(|c| sel.clocks.iter().position(|&x| x == c))
                        .map_or(rt::NO_CLASS, |c| c as u32),
                });
            }
            let mut then = vec![p.write.clone()];
            if !keep {
                then.push(Op::Lit {
                    dst: Ref::Loc(p.flag),
                    val: Box::new([0]),
                });
            }
            ifs.push(Op::If {
                cond: Ref::Loc(p.flag),
                then,
                else_: Vec::new(),
            });
        }
        ifs.push(Op::Lit {
            dst: Ref::Loc(*any),
            val: Box::new([0]),
        });
        writes.entry(gate).or_default().push(r.op(&Op::If {
            cond: Ref::Loc(*any),
            then: ifs,
            else_: Vec::new(),
        }));
    }
    for (w, ops) in writes {
        edge.push(codegen::Step::Gate(w));
        edge.push(codegen::Step::Ops(ops));
    }
    edge.extend(copies(false));
    let err = n;
    n += 2;
    // The log of the notes that fired in a run: a count, then a site per note (each runs at most
    // once per run) and one more the code writes past a full log.
    let notes = checks
        .iter()
        .filter(|c| matches!(c.0, lower::Check::Note(_)))
        .count() as u32;
    let log = n;
    n += 2 + notes;
    // The save words of each loop group.
    for g in &mut groups {
        g.1 = n;
        n += g.0.iter().map(|w| w.1).sum::<u32>();
    }
    // Each guard's copy of its inputs and its word saying what its last run was.
    let guards: Vec<codegen::Guard> = (guards.into_inner().into_iter())
        .map(|(name, ins, outs)| {
            let copy = n;
            let at: Vec<u32> = (ins.iter())
                .scan(0, |a, x| {
                    *a += x.1;
                    Some(*a - x.1)
                })
                .collect();
            n += ins.iter().map(|x| x.1).sum::<u32>() + 1;
            codegen::Guard {
                name,
                ins,
                at,
                copy,
                flag: n - 1,
                outs,
            }
        })
        .collect();
    let mut keep = vec![false; n as usize];
    keep[..signal_words as usize].fill(true);
    for &(w, c) in &state {
        keep[w as usize..(w + c) as usize].fill(true);
    }
    state.push((0, signal_words));
    let top = (e.hier.scopes.iter())
        .position(|s| s.parent.is_none())
        .ok_or("the design has no top-level instance")?;
    let label = e.hier.dotted(yavs_elab::ScopeId(top as u32));
    // Each shape on virtual words: its leader's blocks one after the other.
    let shapes: Vec<codegen::Shape> = (leaders.iter())
        .map(|(body, bl, r)| {
            let mut v = Vec::new();
            let mut at = 0;
            for &(_, _, n) in bl {
                v.push((at, n));
                at += n;
            }
            let place = |b: Block| {
                let j = (bl.iter())
                    .position(|x| x.0 == b)
                    .expect("every block the body addresses is listed");
                v[j].0
            };
            let src = |x: Ref| match x {
                Ref::Sig(k) => Ref::Loc(place(Block::Cur(k))),
                Ref::Loc(s) => Ref::Loc(place(Block::Locals) + s),
                Ref::Edge => Ref::Loc(place(Block::Edge)),
                Ref::Oth => Ref::Loc(place(Block::Oth)),
            };
            let dst = |x: Ref| match x {
                Ref::Sig(k) if r.clocked && next.contains_key(&k) => {
                    Ref::Loc(place(Block::Next(k)))
                }
                x => src(x),
            };
            codegen::Shape {
                ops: body.iter().map(|op| op.map(&src, &dst, &|x| x)).collect(),
                keep: (bl.iter())
                    .flat_map(|&(_, w, n)| keep[w as usize..(w + n) as usize].iter().copied())
                    .collect(),
                blocks: v,
            }
        })
        .collect();
    let (settle, units): (Vec<codegen::Step>, Vec<u32>) = settle.into_iter().unzip();
    let (mut code, mut plan, made) = codegen::compile(
        &settle,
        &units,
        &edge,
        &init_fn,
        &bodies,
        &shapes,
        groups,
        guards,
        &keep,
        signal_words as usize,
        err,
        (log, notes),
        &label,
    )?;
    if code.skip_check {
        // The whole blocks of the signals each step's members write.
        plan.settle_outs = (code.settle_units.iter())
            .map(|us| {
                let ks = std::cell::RefCell::new(BTreeSet::new());
                for &u in us {
                    let b = &sel.members[u as usize].1;
                    ks.borrow_mut().extend(b.writes.iter().map(|&l| ksig(l)));
                    let note = |r: Ref| {
                        if let Ref::Sig(k) = r {
                            ks.borrow_mut().insert(k);
                        }
                        r
                    };
                    for op in &b.ops {
                        op.map(&|r| r, &note, &|x| x);
                    }
                }
                let ks = ks.into_inner();
                (ks.iter())
                    .filter_map(|k| cur.get(k).map(|&w| (w, codegen::words(klen(*k)))))
                    .collect()
            })
            .collect();
        code.settle_outs = plan.settle_outs.clone();
    }
    let (cone, class_cones, class_members, clocked_cones) =
        cones(sel, nets, &code.settle_units, &named);
    // `rt::Skip`: per clocked member its words, its cone and the clocked members its writes reach
    // (those reading what it or a member of its cone writes).
    let clocked: Vec<usize> = (0..sel.members.len())
        .filter(|&k| sel.members[k].1.kind == Kind::Clocked && track_at[k].is_some())
        .collect();
    let cw = clocked.len().div_ceil(64);
    let bit = |v: &mut [u64], i: usize| v[i / 64] |= 1 << (i % 64);
    let mut readers: BTreeMap<LeafId, Vec<usize>> = BTreeMap::new();
    for (ci, &k) in clocked.iter().enumerate() {
        for l in member_leaves(sel, nets, &named, k, true) {
            readers.entry(l).or_default().push(ci);
        }
    }
    let read_by = |ls: &mut dyn Iterator<Item = LeafId>| -> Vec<u64> {
        let mut v = vec![0; cw];
        for l in ls {
            for &c in readers.get(&l).into_iter().flatten() {
                bit(&mut v, c);
            }
        }
        v
    };
    let member_cr: Vec<Vec<u64>> = if clocked.is_empty() {
        Vec::new()
    } else {
        (0..sel.members.len())
            .map(|k| read_by(&mut member_leaves(sel, nets, &named, k, false).into_iter()))
            .collect()
    };
    // What a source reaches: `own`, and the readers of what the members `ms` write.
    let reach_of = |mut own: Vec<u64>, ms: &[u64]| -> Vec<u64> {
        for (j, &w) in ms.iter().enumerate() {
            let mut w = w;
            while w != 0 {
                let k = 64 * j + w.trailing_zeros() as usize;
                w &= w - 1;
                own.iter_mut().zip(&member_cr[k]).for_each(|(a, b)| *a |= b);
            }
        }
        own
    };
    let sparse = |v: &[u64]| -> rt::Sparse {
        (v.iter().enumerate())
            .filter(|x| *x.1 != 0)
            .map(|(j, &w)| (j as u32, w))
            .collect()
    };
    let mut skip = rt::Skip {
        classes: vec![vec![0; cw].into_boxed_slice(); sel.clocks.len()],
        always: vec![0; cw].into_boxed_slice(),
        ..Default::default()
    };
    let mut seen_cone: std::collections::HashMap<rt::Sparse, u32> = Default::default();
    let mut seen_reach: std::collections::HashMap<rt::Sparse, u32> = Default::default();
    let intern = |v: rt::Sparse,
                  seen: &mut std::collections::HashMap<rt::Sparse, u32>,
                  all: &mut Vec<rt::Sparse>| {
        *seen.entry(v.clone()).or_insert_with(|| {
            all.push(v);
            all.len() as u32 - 1
        })
    };
    let cone_of: Vec<&Cone> = (0..sel.members.len())
        .filter(|&k| sel.members[k].1.kind == Kind::Clocked)
        .zip(&clocked_cones)
        .filter(|(k, _)| track_at[*k].is_some())
        .map(|x| x.1)
        .collect();
    for (ci, &k) in clocked.iter().enumerate() {
        let (t, base) = (track_at[k].expect("tracked"), rs[k].base);
        let run = if skips(k) { base + t + 1 } else { u32::MAX };
        if run == u32::MAX {
            bit(&mut skip.always, ci);
        }
        match class_word(k).and_then(|w| edge_words.iter().position(|&x| x == w)) {
            Some(c) => bit(&mut skip.classes[c], ci),
            None => skip.classes.iter_mut().for_each(|c| bit(c, ci)),
        }
        let (st, ms) = cone_of[ci];
        let c = intern(sparse(st), &mut seen_cone, &mut skip.cones);
        let h = intern(
            sparse(&reach_of(member_cr[k].clone(), ms)),
            &mut seen_reach,
            &mut skip.reach,
        );
        skip.tracks.push((run, base + t, c, h));
    }
    if stats {
        eprintln!(
            "cycle stats: {} clocked bodies tracked, {} always run; {} cones, {} reaches",
            skip.tracks.len(),
            skip.always.iter().map(|w| w.count_ones()).sum::<u32>(),
            skip.cones.len(),
            skip.reach.len()
        );
    }
    plan.run_words = (skip.tracks.iter())
        .map(|t| t.0)
        .filter(|&w| w != u32::MAX)
        .collect();
    code.run_words = plan.run_words.clone();
    if stats {
        // The settle steps each edge class's clocked members reach: an edge runs only those and
        // the steps its inputs reach.
        let n = code.settle_units.len();
        let ops = |i: usize| -> usize {
            (code.settle_units[i].iter())
                .map(|&u| sel.members[u as usize].1.ops.len())
                .sum()
        };
        let total: usize = (0..n).map(ops).sum();
        for (ci, (c, m)) in sel.clocks.iter().zip(class_cones.iter()).enumerate() {
            let hit: Vec<usize> = (0..n).filter(|i| m[i / 64] >> (i % 64) & 1 == 1).collect();
            let mine: Vec<usize> = (0..sel.members.len())
                .filter(|&k| {
                    let b = &sel.members[k].1;
                    b.kind == Kind::Clocked
                        && b.clock
                            .and_then(|x| sel.clocks.iter().position(|&y| y == x))
                            == Some(ci)
                })
                .collect();
            eprintln!(
                "cycle stats: edge class {}: {} clocked members, {} operations; {} members without a class",
                e.leaf_name(c.leaf),
                mine.len(),
                mine.iter()
                    .map(|&k| sel.members[k].1.ops.len())
                    .sum::<usize>(),
                (sel.members.iter())
                    .filter(|(_, b)| b.kind == Kind::Clocked && b.clock.is_none())
                    .count(),
            );
            eprintln!(
                "cycle stats: edge class {} {}: its members reach {} of {n} settle steps, {} of {total} operations",
                if c.rising { "rising" } else { "falling" },
                e.leaf_name(c.leaf),
                hit.len(),
                hit.iter().map(|&i| ops(i)).sum::<usize>(),
            );
        }
    }
    let whole = |k: u32| {
        let s = yavs_kernel::SignalId(k);
        s.slice(0, nets.signal_len(s))
    };
    // A null array has no elements to read or drive.
    let all: Vec<_> = (cur.iter())
        .filter(|&(&k, _)| !logic(k) || nets.signal_len(yavs_kernel::SignalId(k)) > 0)
        .map(|(&k, &s)| rt::Read {
            r: whole(k),
            slot: s,
            lo: 0,
            len: nets.signal_len(yavs_kernel::SignalId(k)),
            bytes: bytes(k),
            now: true,
        })
        .collect();
    // Inputs leaf by leaf (a kernel signal may also hold leaves the region drives), a leaf of
    // its own signal in chunks of one word, grouped by kernel signal: each chunk is read when the
    // kernel counted a change of it.
    // The inputs whose change is settled and exported at once: the resets from outside and the
    // wakes with a combinational path to something outside can see (`Selection::at_once`).
    // `YAVS_CYCLE_EDGE_OUTPUTS=1`, an experiment: only the resets, so the outputs change at
    // clock edges only.
    let edge_outputs = std::env::var_os("YAVS_CYCLE_EDGE_OUTPUTS").is_some_and(|v| v == "1");
    let now: std::collections::BTreeSet<LeafId> = (sel.members.iter())
        .filter_map(|(_, b)| b.reset.map(|r| r.0))
        .chain(sel.at_once.keys().copied().filter(|_| !edge_outputs))
        .collect();
    let mut by_sig: BTreeMap<u32, Vec<InChunk>> = BTreeMap::new();
    for &l in sel.inputs.iter().filter(|&&l| nets.leaf(l).ty.len > 0) {
        let len = nets.leaf(l).ty.len;
        let at = nets.loc[l.index()];
        let (slot, b) = (cur[&at.signal.0], bytes(at.signal.0));
        let step = if at.base == 0 && b == 0 { 64 } else { len };
        let chunks = by_sig.entry(at.signal.0).or_default();
        for k in (0..len).step_by(step as usize) {
            let part = Range::new(k, step.min(len - k));
            let read = rt::Read {
                r: nets.kref(l, part),
                slot: slot + k / 64,
                lo: at.base,
                len: part.len,
                bytes: b,
                now: now.contains(&l),
            };
            chunks.push((read, nets.kpart(l, part).1, (l, part.offset, part.len)));
        }
    }
    let (mut chunks, mut ins) = (Vec::new(), Vec::new());
    let mut chunk_at: Vec<(LeafId, u32, u32)> = Vec::new();
    // One image per group of input signals of about `IMAGE_CHUNKS` chunks: a wake asks each
    // group's write count, and compares the chunk counts only of a group that was written.
    let mut items: Vec<yavs_kernel::ImageItem> = Vec::new();
    let mut groups = by_sig.into_iter().peekable();
    while let Some((k, rs)) = groups.next() {
        for (_, r, _) in &rs {
            let r = *r;
            items.push(yavs_kernel::ImageItem {
                sig: yavs_kernel::SignalId(k),
                k: r.offset,
                len: r.len,
                voff: 8 * items.len() as u32,
                vsize: 8,
                kind: yavs_kernel::ImageKind::Count,
            });
        }
        for c in rs {
            chunks.push(c.0);
            chunk_at.push(c.2);
        }
        if items.len() >= IMAGE_CHUNKS || groups.peek().is_none() {
            let n = items.len() as u32;
            ins.push(rt::InSig {
                spec: yavs_kernel::ImageSpec {
                    size: 8 * n,
                    items: std::mem::take(&mut items).into(),
                },
                at: chunks.len() as u32 - n,
                n,
                id: None,
                seen: 0,
            });
        }
    }
    // Each class: its clock element, the edge test of `Ctx::edge_by`, its word.
    let clocks: Vec<(yavs_kernel::SigRef, u8, usize)> = (sel.clocks.iter().zip(&edge_words))
        .map(|(c, &w)| {
            let kind = u8::from(c.rising) | if c.legacy { EDGE_LEVEL } else { 0 };
            (nets.kref(c.leaf, Range::new(c.elem, 1)), kind, w as usize)
        })
        .collect();
    let leaf_ref = |l: LeafId| nets.kref(l, Range::new(0, nets.leaf(l).ty.len));
    let mut waits: Vec<yavs_kernel::SigRef> = Vec::new();
    for c in &clocks {
        if !waits.contains(&c.0) {
            waits.push(c.0);
        }
    }
    waits.extend(
        (sel.wakes.iter())
            .filter(|&&l| nets.leaf(l).ty.len > 0)
            .map(|l| leaf_ref(*l)),
    );
    // The asynchronous resets from outside: their events wake the region too (one made inside
    // changes only in the region's own settle).
    let resets: std::collections::BTreeSet<(LeafId, u32, u8)> = (sel.members.iter())
        .filter_map(|(_, b)| b.reset)
        .filter(|r| !sel.driven.contains(&r.0))
        .collect();
    for &(l, el, _) in &resets {
        waits.push(nets.kref(l, Range::new(el, 1)));
    }
    // Only clocks wake it: a wake without an edge has nothing to do.
    let clocks_only = sel.wakes.is_empty() && resets.is_empty();
    let one_reset = match resets.iter().collect::<Vec<_>>()[..] {
        [] => Some(None),
        [&(l, el, code)] => {
            let (s, e) = nets.kelem(l, el);
            Some(Some((s, e, code)))
        }
        _ => None,
    };
    let trigger = match &sel.clocks[..] {
        [c] if sel.wakes.is_empty() => one_reset.map(|reset| {
            let (signal, elem) = nets.kelem(c.leaf, c.elem);
            EdgeTrigger {
                signal,
                elem,
                rising: c.rising,
                legacy: c.legacy,
                reset,
            }
        }),
        _ => None,
    };
    // The leaf of each driver declaration, for the exports.
    let decls: Vec<LeafId> = nets.drivers.iter().map(|d| d.leaf).collect();
    // Exported kernel signals none of whose leaves anything outside waits on: candidates for an
    // update without an event (the kernel decides at run time, `Builder::quiet`).
    let waited: std::collections::BTreeSet<u32> = sel.waited.iter().map(|&l| ksig(l)).collect();
    let quiet: Vec<u32> = (sel.outputs.iter())
        .map(|&l| ksig(l))
        .filter(|k| !waited.contains(k))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let names: BTreeMap<usize, String> = (checks.iter())
        .map(|(_, _, pi)| (*pi, e.hier.proc_label(&e.procs[*pi], yavs_elab::Case::Lrm)))
        .collect();
    let checks: Vec<rt::Site> = checks
        .into_iter()
        .map(|(c, at, pi)| (c, at, pi, names[&pi].clone()))
        .collect();
    // The reports of the calls members fold, by member (A22).
    let reports: Vec<(usize, u8, String)> = (sel.members.iter())
        .flat_map(|(pi, b)| b.reports.iter().map(|(s, t)| (*pi, *s, t.clone())))
        .collect();
    let sig_bytes: BTreeMap<u32, u32> = cur.keys().map(|&k| (k, bytes(k))).collect();
    // The elements of `BIT` and `BOOLEAN` leaves of logic signals, which the kernel keeps as
    // logic too: the second image drives no `'U'` there.
    let two_valued: Vec<(u32, u32, u32)> = (cur.keys())
        .filter(|&&k| logic(k) && region::exact_meta())
        .flat_map(|&k| {
            let (a, z) = nets.packs[k as usize];
            (a.index()..z.index()).map(move |i| (k, LeafId(i as u32)))
        })
        .filter(|&(_, l)| two_valued_leaf(db, e, l))
        .map(|(k, l)| (k, nets.loc[l.index()].base, nets.leaf(l).ty.len))
        .collect();
    let sig_len: BTreeMap<u32, u32> = (cur.keys())
        .map(|&k| (k, nets.signal_len(yavs_kernel::SignalId(k))))
        .collect();
    // Per input chunk: the steps of the settle a change of it can reach, and the members.
    let (cones, chunk_members, chunk_cone) = {
        let mut by_leaf: BTreeMap<LeafId, u32> = BTreeMap::new();
        let (mut cones, mut members): (rt::Marks, rt::Marks) = Default::default();
        let chunk_cone: Vec<u32> = (chunk_at.iter())
            .map(|&(l, _, _)| {
                *by_leaf.entry(l).or_insert_with(|| {
                    let (c, m) = cone(l);
                    if !clocked.is_empty() {
                        let own = read_by(&mut std::iter::once(l));
                        let h =
                            intern(sparse(&reach_of(own, &m)), &mut seen_reach, &mut skip.reach);
                        skip.chunk.push(h);
                    }
                    cones.push(c);
                    members.push(m);
                    cones.len() as u32 - 1
                })
            })
            .collect();
        (cones, members, chunk_cone)
    };
    // The image words each member writes: the export compares only the outputs the members a
    // wake's inputs and edges reach write.
    let wrote = |k: usize| -> Vec<(u32, u32)> {
        let b = &sel.members[k].1;
        let mut v = leaf_words(&b.writes, None);
        let listed: BTreeSet<u32> = b.reads.iter().chain(&b.writes).map(|&l| ksig(l)).collect();
        for k in named[k][1].difference(&listed) {
            if let Some(&slot) = cur.get(k) {
                v.push((slot, codegen::words(klen(*k))));
            }
        }
        v
    };
    let member_words: Vec<Vec<(u32, u32)>> = (0..sel.members.len()).map(wrote).collect();
    let mut members: Vec<usize> = sel.members.iter().map(|m| m.0).collect();
    members.sort_unstable();
    let built = Built {
        name: format!("{}.cycle_region", label),
        members,
        trigger,
        quiet,
        summary: summary.to_string(),
        warning: dropped_warning(db, e, sel),
        code: plan,
        outputs: sel.outputs.clone(),
        decls,
        cur,
        sig_bytes,
        two_valued,
        sig_len,
        ram_writes,
        member_words,
        cones,
        chunk_members,
        class_cones,
        class_members,
        skip,
        words: n as usize,
        init,
        all,
        chunks,
        ins,
        clocks,
        clocks_only,
        waits,
        checks,
        err: err as usize,
        log: (log as usize, notes as usize),
        reports,
        uninit,
        useed,
        state,
        regs: reg_runs(regs),
        oth: region::exact_meta().then_some(oth as usize),
        seeds: (sel.members.iter()).any(|(_, b)| has_seed(&b.ops) || has_seed(&b.init_ops)),
        chunk_cone,
        chunk_at,
        class_names: (sel.clocks.iter())
            .map(|c| {
                let edge = if c.rising { "rising" } else { "falling" };
                format!("{edge} {}", e.leaf_name(c.leaf))
            })
            .collect(),
        ops: (sel.members.iter())
            .map(|(_, b)| b.ops.iter().map(codegen::weight_of).sum::<usize>() as u64)
            .collect(),
    };
    Ok((built, code, made))
}

/// What building a region gives besides its machine code, kept with the code in the analysis
/// cache (`store`): a warm run makes the region from it without the members' operations.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Built {
    name: String,
    members: Vec<usize>,
    trigger: Option<EdgeTrigger>,
    quiet: Vec<u32>,
    /// The summary line of `--cycle` ([`Plan::summary`]).
    summary: String,
    /// W4101's headline and notes ([`Plan::warning`]).
    warning: Option<(String, Vec<String>)>,
    pub(crate) code: codegen::CodePlan,
    outputs: BTreeSet<LeafId>,
    decls: Vec<LeafId>,
    cur: BTreeMap<u32, u32>,
    sig_bytes: BTreeMap<u32, u32>,
    /// The `BIT` and `BOOLEAN` leaves of logic signals: signal, first element, elements.
    two_valued: Vec<(u32, u32, u32)>,
    sig_len: BTreeMap<u32, u32>,
    ram_writes: BTreeMap<u32, Vec<rt::RamWrite>>,
    /// Per member: the image words it writes.
    member_words: Vec<Vec<(u32, u32)>>,
    cones: Vec<Box<[u64]>>,
    /// Per cone and per edge class: the members it reaches (with a class's own clocked ones).
    chunk_members: Vec<Box<[u64]>>,
    class_cones: Vec<Box<[u64]>>,
    class_members: Vec<Box<[u64]>>,
    skip: rt::Skip,
    words: usize,
    init: Vec<(u32, u64)>,
    all: Vec<rt::Read>,
    chunks: Vec<rt::Read>,
    ins: Vec<rt::InSig>,
    clocks: Vec<(yavs_kernel::SigRef, u8, usize)>,
    clocks_only: bool,
    waits: Vec<yavs_kernel::SigRef>,
    /// Kept apart in the cache (`store`), decoded only when a check fails ([`rt::Sites`]).
    #[serde(skip)]
    pub(crate) checks: Vec<rt::Site>,
    err: usize,
    log: (usize, usize),
    reports: Vec<(usize, u8, String)>,
    uninit: u64,
    /// The words of clocked variables without an initial value (first, elements), and the
    /// words that hold state (first, count): the second image of `--cycle-exact-meta`.
    useed: Vec<(u32, u32)>,
    state: Vec<(u32, u32)>,
    /// The bits clocked members keep between edges: runs of words (first, count, bits).
    regs: Vec<(u32, u32, u64)>,
    /// Under `--cycle-exact-meta`: the word of `Ref::Oth`; whether the code has `Op::Seed`.
    oth: Option<usize>,
    seeds: bool,
    chunk_cone: Vec<u32>,
    /// Per input chunk: its leaf and elements (the names of `YAVS_CYCLE_STATS`).
    chunk_at: Vec<(LeafId, u32, u32)>,
    /// `YAVS_CYCLE_STATS`: the name of each edge class, the operations of each member.
    class_names: Vec<String>,
    ops: Vec<u64>,
}

/// The region of `b` with check sites `sites` running `code`: its kernel process made from `b`
/// alone.
fn finish(
    e: &Elaborated,
    b: Built,
    sites: rt::Sites,
    code: codegen::Code,
    metas: Arc<Metas>,
    reporter: Reporter,
) -> Region {
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    let import_check = std::env::var_os("YAVS_CYCLE_IMPORT_CHECK").is_some_and(|v| v == "1");
    let code = Arc::new(code);
    let class_cones = Arc::new(b.class_cones);
    let skip = Arc::new(b.skip);
    let mark_words = code.settle_units.len().div_ceil(64);
    if stats {
        let ops: Vec<u64> = b.ops;
        let mut steps_of = vec![0u32; ops.len()];
        for &u in code.settle_units.iter().flatten() {
            steps_of[u as usize] += 1;
        }
        let own = (code.settle_units.iter())
            .map(|us| {
                let (one, shared): (Vec<u32>, Vec<u32>) =
                    us.iter().partition(|&&u| steps_of[u as usize] == 1);
                (one.iter().map(|&u| ops[u as usize]).sum(), shared)
            })
            .collect();
        *metas.stats.lock().expect("not poisoned") = Some(rt::Stats {
            wakes: 0,
            edges: 0,
            chunks: (b.chunk_at.iter())
                .map(|&(l, o, n)| (format!("{} ({o}:{n})", e.leaf_name(l)), 0))
                .collect(),
            code: code.clone(),
            classes: b.class_names.into_iter().map(|n| (n, 0)).collect(),
            all: (steps_of.iter().zip(&ops))
                .filter(|x| *x.0 > 0)
                .map(|x| x.1)
                .sum(),
            own,
            seen: vec![u64::MAX; ops.len()],
            ops,
            settled: [[0; 2]; 2],
            ex: [0; rt::EX_N],
        });
    }
    let (outputs, decls, cur) = (b.outputs, b.decls, b.cur);
    let (sig_bytes, sig_len, ram_writes) = (b.sig_bytes, b.sig_len, b.ram_writes);
    let (member_words, chunk_members, class_members) =
        (b.member_words, b.chunk_members, b.class_members);
    let (all, chunks, ins, clocks) = (b.all, b.chunks, b.ins, b.clocks);
    let waits = rt::held_waits(&clocks, &b.waits);
    let (words, init, clocks_only, err, log, uninit) =
        (b.words, b.init, b.clocks_only, b.err, b.log, b.uninit);
    let (useed, state) = (Arc::new(b.useed), Arc::new(b.state));
    let two_valued = b.two_valued;
    let regs = Arc::new(b.regs);
    // The second image of `--cycle-exact-meta` ('U' at start, A38): its region process is the
    // one with the rail's code; the default's has none of it.
    let rail = region::exact_meta().then(rt::rail_knob);
    let checks = Arc::new(sites);
    let reports = Arc::new(b.reports);
    let cones = Arc::new(b.cones);
    let chunk_cone = Arc::new(b.chunk_cone);
    let make: yavs_elab::RegionMake = Arc::new(move |ds: &[RegionDriver]| {
        // One export per kernel driver with an exported leaf: its range of the signal's words.
        let mut outs: BTreeMap<u32, rt::Out> = BTreeMap::new();
        for d in (ds.iter()).filter(|d| outputs.contains(&decls[d.decl]) && d.range.len > 0) {
            let Some(&slot) = cur.get(&d.signal.0) else {
                return Err(SimError::new(yavs_kernel::ErrorKind::Internal(
                    "cycle mode: an exported signal without words".into(),
                )));
            };
            // Integers: the image keeps the signal's elements from the right.
            let slot = match sig_bytes[&d.signal.0] {
                0 => slot,
                _ => slot + sig_len[&d.signal.0] - d.range.offset - d.range.len,
            };
            outs.entry(d.driver.0).or_insert(rt::Out {
                driver: d.driver,
                range: d.range,
                slot,
                bytes: sig_bytes[&d.signal.0],
                last: 0,
                wide: Vec::new(),
                ram: ram_writes.get(&d.signal.0).cloned().unwrap_or_default(),
            });
        }
        // Per driver, the elements of its range of `BIT` and `BOOLEAN` leaves (from its first).
        let mut two: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
        for d in ds.iter().filter(|d| outs.contains_key(&d.driver.0)) {
            for &(_, base, len) in two_valued.iter().filter(|t| t.0 == d.signal.0) {
                let (a, z) = (base.max(d.range.offset), (base + len).min(d.range.end()));
                if a < z {
                    two.entry(d.driver.0)
                        .or_default()
                        .push((a - d.range.offset, z - a));
                }
            }
        }
        let mut outs: Vec<rt::Out> = outs.into_values().collect();
        let (spans, shadow) = rt::spans(&mut outs);
        if stats {
            eprintln!(
                "cycle stats: {} outputs ({} memories) in {} spans of {shadow} words",
                outs.len(),
                outs.iter().filter(|o| !o.ram.is_empty()).count(),
                spans.len()
            );
        }
        let rams: Vec<Vec<u32>> = (outs.iter())
            .filter(|o| !o.ram.is_empty())
            .map(|o| o.ram.iter().map(|w| w.class).collect())
            .collect();
        let (chunk_spans, class_spans) =
            rt::span_marks(&spans, &member_words, &chunk_members, &class_members, &rams);
        let touched = (spans.len() + rams.len()).div_ceil(64);
        let mut img = vec![0u64; words];
        for &(w, v) in &init {
            img[w as usize] = v;
        }
        let rail_cfg = match &rail {
            Some(Ok(c)) => *c,
            Some(Err(m)) => {
                return Err(SimError::new(yavs_kernel::ErrorKind::Internal(m.clone())));
            }
            None => None,
        };
        macro_rules! region_proc {
            ($rail:literal) => {
                Box::new(rt::RegionProc::<$rail> {
                    code: code.clone(),
                    img,
                    leaves: all.clone(),
                    seen: vec![0; chunks.len()],
                    chunks: chunks.clone(),
                    ins: ins.clone(),
                    looked: false,
                    check: import_check,
                    exported: false,
                    pending: false,
                    outs,
                    spans,
                    shadow: vec![0; shadow],
                    clocks: clocks.clone(),
                    clocks_only,
                    waits: waits.clone(),
                    started: false,
                    checks: checks.clone(),
                    err,
                    log,
                    reporter: reporter.clone(),
                    reports: reports.clone(),
                    metas: metas.clone(),
                    uninit,
                    buf: Vec::new(),
                    read_now: Vec::new(),
                    idle: rt::Idle {
                        on: words <= rt::IDLE_WORDS && log.1 == 0 && clocks.len() <= 64,
                        ..Default::default()
                    },
                    dirty: vec![!0; skip.always.len()],
                    ored: (vec![0; skip.cones.len()], vec![0; skip.reach.len()], 0),
                    go: vec![0; skip.always.len()],
                    skip: skip.clone(),
                    cones: cones.clone(),
                    chunk_cone: chunk_cone.clone(),
                    mark: vec![0; mark_words],
                    class_cones: class_cones.clone(),
                    touched: vec![0; touched],
                    chunk_spans,
                    class_spans,
                    stats,
                    ex: [0; rt::EX_N],
                    rail: None,
                    rail_cfg,
                    useed: useed.clone(),
                    state: state.clone(),
                    two: two.clone(),
                    regs: regs.clone(),
                    oth: b.oth,
                    seeds: b.seeds,
                })
            };
        }
        Ok(if rail_cfg.is_some() {
            region_proc!(true) as Box<dyn yavs_kernel::Process>
        } else {
            region_proc!(false)
        })
    });
    Region {
        name: b.name,
        members: b.members,
        trigger: b.trigger,
        quiet: b.quiet,
        make,
    }
}

/// Words with their bits as runs of adjacent words with the same bits (memories are one run).
fn reg_runs(regs: BTreeMap<u32, u64>) -> Vec<(u32, u32, u64)> {
    let mut v: Vec<(u32, u32, u64)> = Vec::new();
    for (w, m) in regs {
        match v.last_mut() {
            Some(r) if r.0 + r.1 == w && r.2 == m => r.1 += 1,
            _ => v.push((w, 1, m)),
        }
    }
    v
}

/// Leaf `l` is of `BIT` or `BOOLEAN` (or an array of them): logic in the kernel, never `'U'`.
/// (Logic subtypes with a range, `X01`, keep their processes out of the region.)
fn two_valued_leaf(db: &Db, e: &Elaborated, l: LeafId) -> bool {
    use yavs_elab::Shape;
    let mut sh = &e.nets.obj(e.nets.leaf(l).obj).shape;
    let mut path = e.nets.leaf(l).path.iter();
    let ty = loop {
        match sh {
            Shape::Array { elem, .. } => sh = elem,
            Shape::Record { fields, .. } => {
                match path.next().and_then(|&f| fields.get(f as usize)) {
                    Some((_, f)) => sh = f,
                    None => return false,
                }
            }
            Shape::Scalar { ty, .. } => break *ty,
        }
    };
    let wk = &db.well_known;
    Some(ty) == wk.bit || Some(ty) == wk.boolean
}

/// The steps of the settle and the members a change reaches, as bit sets.
type Cone = (Box<[u64]>, Box<[u64]>);

/// For a leaf the region reads from outside: the members a change of it can reach (as a bit set),
/// through the members that read it and, from what they drive, the members that read that, and so
/// on, and the steps of the settle (`settle_units`) that run them. Reads and writes of a member are
/// its listed leaves and every leaf of a signal its operations name without listing one. Also per
/// edge class: the steps the writes of its clocked members reach (a member with no class runs at
/// every edge; a reset branch reads what it holds on), and those members with the ones they reach.
fn cones<'a>(
    sel: &'a region::Selection,
    nets: &'a yavs_elab::NetGraph,
    settle_units: &[Vec<u32>],
    named: &[[BTreeSet<u32>; 2]],
) -> (
    impl Fn(LeafId) -> Cone + 'a,
    rt::Marks,
    rt::Marks,
    Vec<Cone>,
) {
    let n = sel.members.len();
    let leaves = |k: usize, reads: bool| member_leaves(sel, nets, named, k, reads);
    let (words, mw) = (settle_units.len().div_ceil(64), n.div_ceil(64));
    // Each member's own steps.
    let mut own: Vec<Vec<u64>> = vec![vec![0; words]; n];
    for (i, us) in settle_units.iter().enumerate() {
        for &u in us {
            own[u as usize][i / 64] |= 1 << (i % 64);
        }
    }
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; n];
    for &k in &sel.settle {
        if !std::mem::replace(&mut seen[k], true) {
            order.push(k);
        }
    }
    let io: Vec<(usize, Vec<LeafId>, Vec<LeafId>)> = (order.iter().rev())
        .map(|&k| {
            let (r, w) = (leaves(k, true), leaves(k, false));
            (k, r.into_iter().collect(), w.into_iter().collect())
        })
        .collect();
    // The members reading each leaf; a member reaches itself and the members reading what it
    // writes. In strongly connected components (loop groups, and leaves coarser than the
    // settle's elements), each finished after those it reaches (Tarjan), so each is done once.
    let mut readers: std::collections::HashMap<LeafId, Vec<u32>> = Default::default();
    for (i, (_, r, _)) in io.iter().enumerate() {
        for l in r {
            readers.entry(*l).or_default().push(i as u32);
        }
    }
    let succ: Vec<Vec<u32>> = (io.iter())
        .map(|(_, _, w)| {
            let mut v: Vec<u32> = (w.iter())
                .flat_map(|l| readers.get(l).into_iter().flatten().copied())
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        })
        .collect();
    let m = io.len();
    let (mut index, mut low, mut on) = (vec![u32::MAX; m], vec![0u32; m], vec![false; m]);
    let (mut comp, mut comps) = (vec![u32::MAX; m], Vec::<Vec<u32>>::new());
    let (mut stack, mut next) = (Vec::<u32>::new(), 0u32);
    for root in 0..m {
        if index[root] != u32::MAX {
            continue;
        }
        // (node, position in its successors)
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root as u32);
        on[root] = true;
        while let Some(&mut (v, ref mut at)) = call.last_mut() {
            if let Some(&w) = succ[v].get(*at) {
                *at += 1;
                let w = w as usize;
                if index[w] == u32::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w as u32);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut c = Vec::new();
                while let Some(w) = stack.pop() {
                    on[w as usize] = false;
                    comp[w as usize] = comps.len() as u32;
                    c.push(w);
                    if w as usize == v {
                        break;
                    }
                }
                comps.push(c);
            }
        }
    }
    // Per component, in the order finished: its steps and members, and those it reaches.
    let mut reach: Vec<(Vec<u64>, Vec<u64>)> = Vec::with_capacity(comps.len());
    for (ci, c) in comps.iter().enumerate() {
        let (mut st, mut ms) = (vec![0; words], vec![0; mw]);
        for &i in c {
            let k = io[i as usize].0;
            st.iter_mut().zip(&own[k]).for_each(|(a, b)| *a |= b);
            ms[k / 64] |= 1 << (k % 64);
            for &j in &succ[i as usize] {
                let cj = comp[j as usize] as usize;
                if cj != ci {
                    let (xs, xm) = &reach[cj];
                    st.iter_mut().zip(xs).for_each(|(a, b)| *a |= b);
                    ms.iter_mut().zip(xm).for_each(|(a, b)| *a |= b);
                }
            }
        }
        reach.push((st, ms));
    }
    // What a leaf reaches: what its readers reach.
    let of = move |l: &LeafId, st: &mut [u64], ms: &mut [u64]| {
        for &j in readers.get(l).into_iter().flatten() {
            let (xs, xm) = &reach[comp[j as usize] as usize];
            st.iter_mut().zip(xs).for_each(|(a, b)| *a |= b);
            ms.iter_mut().zip(xm).for_each(|(a, b)| *a |= b);
        }
    };
    // Per clocked member: the steps and members its writes reach.
    let clocked: Vec<Cone> = (0..n)
        .filter(|&k| sel.members[k].1.kind == Kind::Clocked)
        .map(|k| {
            let (mut st, mut ms) = (vec![0; words], vec![0; mw]);
            for l in leaves(k, false) {
                of(&l, &mut st, &mut ms);
            }
            (st.into_boxed_slice(), ms.into_boxed_slice())
        })
        .collect();
    let (mut classes, mut class_members) = (Vec::new(), Vec::new());
    for c in 0..sel.clocks.len() {
        let (mut st, mut reach, mut mine) = (vec![0; words], vec![0; mw], vec![0; mw]);
        for (k, (_, b)) in sel.members.iter().enumerate() {
            let class = b
                .clock
                .and_then(|x| sel.clocks.iter().position(|&y| y == x));
            if b.kind != Kind::Clocked || class.is_some_and(|x| x != c) {
                continue;
            }
            mine[k / 64] |= 1 << (k % 64);
            for l in leaves(k, false) {
                of(&l, &mut st, &mut reach);
            }
        }
        classes.push(st.into_boxed_slice());
        reach.iter_mut().zip(&mine).for_each(|(a, b)| *a |= b);
        class_members.push(reach.into_boxed_slice());
    }
    let of_leaf = move |l: LeafId| {
        let (mut st, mut ms) = (vec![0; words], vec![0; mw]);
        of(&l, &mut st, &mut ms);
        (st.into_boxed_slice(), ms.into_boxed_slice())
    };
    (of_leaf, classes, class_members, clocked)
}

/// The leaves member `k` reads (or writes): those it lists, and every leaf of a signal its
/// operations name without listing one.
fn member_leaves(
    sel: &region::Selection,
    nets: &yavs_elab::NetGraph,
    named: &[[BTreeSet<u32>; 2]],
    k: usize,
    reads: bool,
) -> BTreeSet<LeafId> {
    let ksig = |l: LeafId| nets.loc[l.index()].signal.0;
    let b = &sel.members[k].1;
    let listed: BTreeSet<u32> = b.reads.iter().chain(&b.writes).map(|&l| ksig(l)).collect();
    let mut v: BTreeSet<LeafId> = if reads {
        b.reads.clone()
    } else {
        b.writes.clone()
    };
    for k in named[k][usize::from(!reads)].difference(&listed) {
        let (a, z) = nets.packs[*k as usize];
        v.extend((a.0..z.0).map(LeafId));
    }
    v
}

/// Per member: the signals its operations read and those they write by name (`Ref::Sig`).
fn named_sigs(sel: &region::Selection) -> Vec<[BTreeSet<u32>; 2]> {
    (sel.members.iter())
        .map(|(_, b)| {
            let n = std::cell::RefCell::new([BTreeSet::new(), BTreeSet::new()]);
            let note = |i: usize, r: Ref| {
                if let Ref::Sig(k) = r {
                    n.borrow_mut()[i].insert(k);
                }
                r
            };
            for op in &b.ops {
                op.map(&|r| note(0, r), &|r| note(1, r), &|x| x);
            }
            n.into_inner()
        })
        .collect()
}

/// A guard before its words are placed: its name, the words it reads and those it drives.
type GuardSpec = (String, Vec<(u32, u32)>, Vec<(u32, u32)>);

/// An input chunk while the inputs are grouped: its read, its range of the kernel signal, its
/// leaf and elements.
type InChunk = (rt::Read, Range, (LeafId, u32, u32));

/// Operations a combinational member has at least for a guard (`codegen::Guard`), which also
/// needs it to read no more words than it has operations: a smaller member costs less to run than
/// to compare.
const GUARD_MIN: usize = 64;

/// Input spans of a guard at most this many words apart are compared as one.
const GUARD_GAP: u32 = 4;

/// The input chunks one kernel image counts the changes of (`rt::InSig`).
const IMAGE_CHUNKS: usize = 32;

/// A memory write of a clocked member kept back to the commit: the write, from the member's
/// words that hold its index and value, and the word that says it happened (member slots).
struct Pending {
    write: Op,
    flag: u32,
}

/// `ops` with every write of a memory in `rams` replaced by saving its index and value in new
/// member slots from `at` on and setting its flag; the writes, for the commit.
fn defer(
    ops: &[Op],
    rams: &std::collections::BTreeSet<u32>,
    any: u32,
    at: &mut u32,
    out: &mut Vec<Pending>,
) -> Vec<Op> {
    let mut res = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::If { cond, then, else_ } => res.push(Op::If {
                cond: *cond,
                then: defer(then, rams, any, at, out),
                else_: defer(else_, rams, any, at, out),
            }),
            Op::IndexWrite {
                dst: Ref::Sig(k),
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } if rams.contains(k) => {
                let (i, v, f) = (*at, *at + 1, *at + 1 + codegen::words(*w));
                *at = f + 1;
                res.extend([
                    Op::Copy {
                        dst: Ref::Loc(i),
                        src: *idx,
                        len: 64,
                    },
                    Op::Copy {
                        dst: Ref::Loc(v),
                        src: *src,
                        len: *w,
                    },
                    Op::Lit {
                        dst: Ref::Loc(f),
                        val: Box::new([1]),
                    },
                    Op::Lit {
                        dst: Ref::Loc(any),
                        val: Box::new([1]),
                    },
                ]);
                out.push(Pending {
                    write: Op::IndexWrite {
                        dst: Ref::Sig(*k),
                        src: Ref::Loc(v),
                        lo: *lo,
                        n: *n,
                        stride: *stride,
                        w: *w,
                        idx: Ref::Loc(i),
                        left: *left,
                        down: *down,
                        site: *site,
                    },
                    flag: f,
                });
            }
            op => res.push(op.clone()),
        }
    }
    res
}

/// Developers (`YAVS_CYCLE_STATS=1`): members whose code would be the same but for where their
/// signals and slots lie (everything folded into the code the same: constants, widths, offsets
/// within signals), as distinct shapes; the operations one body per shape would compile, also
/// counting only shapes of at least some size; the largest shapes by operations times instances.
fn shapes(db: &Db, e: &Elaborated, sel: &region::Selection, next: &BTreeMap<u32, u32>) {
    let mut by: std::collections::HashMap<ShapeKey, (usize, usize, usize), codegen::KeyBuild> =
        Default::default();
    for (pi, b) in &sel.members {
        let key = shape_key(b, next);
        let w: usize = b.ops.iter().map(codegen::weight_of).sum();
        let e = by.entry(key).or_insert((0, w, *pi));
        e.0 += 1;
    }
    let mut v: Vec<_> = by.values().collect();
    v.sort_by_key(|&&(n, w, pi)| (std::cmp::Reverse(n * w), pi));
    let all: usize = v.iter().map(|&&(n, w, _)| n * w).sum();
    let once: usize = v.iter().map(|&&(_, w, _)| w).sum();
    eprintln!(
        "cycle stats: {} members, {} shapes, {all} operations, {once} with one body per shape",
        sel.members.len(),
        v.len()
    );
    for t in [16, 64, 256, 1024] {
        let saved: usize = (v.iter())
            .filter(|&&&(_, w, _)| w >= t)
            .map(|&&(n, w, _)| (n - 1) * w)
            .sum();
        eprintln!("cycle stats:   shapes of {t} operations or more: {saved} operations saved");
    }
    for &&(n, w, pi) in v.iter().take(8) {
        eprintln!(
            "cycle stats:   {n} x {w} operations, {}",
            region::where_(db, e, pi)
        );
    }
}

/// Members of at least this many operations whose code is the same but for where their words lie
/// share one compiled body; below it a call per member would cost more than the code it saves.
const SHARE_MIN: usize = 256;

/// A place a member's code addresses, which each member of a shape has at an address of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    /// The current words of kernel signal `k`, its next-state words, the member's slots, its
    /// edge word.
    Cur(u32),
    Next(u32),
    Locals,
    Edge,
    Oth,
}

/// Blocks with their first word and length.
type Blocks = Vec<(Block, u32, u32)>;

fn same_kind(a: Block, b: Block) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

/// The body a member runs: a combinational member's operations; a clocked member's, behind its edge
/// word with `test` (several classes and none of its own: the others run behind their class's
/// gate).
fn shared_body(ops: &[Op], clocked: bool, test: bool) -> Vec<Op> {
    if clocked && test {
        vec![Op::If {
            cond: Ref::Edge,
            then: ops.to_vec(),
            else_: Vec::new(),
        }]
    } else {
        ops.to_vec()
    }
}

/// The blocks `ops` (of member `b`, resolved by `r`) address, in order of first use: each with its
/// first word in the image and its length (its slots `extra` more: a flag and a run word).
fn blocks(
    ops: &[Op],
    b: &lower::Body,
    r: &Resolve,
    klen: impl Fn(u32) -> u32,
    extra: u32,
) -> Blocks {
    let seen = std::cell::RefCell::new(Vec::<Block>::new());
    let add = |x: Block| {
        let mut s = seen.borrow_mut();
        if !s.contains(&x) {
            s.push(x);
        }
    };
    let src = |x: Ref| {
        add(match x {
            Ref::Sig(k) => Block::Cur(k),
            Ref::Loc(_) => Block::Locals,
            Ref::Edge => Block::Edge,
            Ref::Oth => Block::Oth,
        });
        x
    };
    let dst = |x: Ref| {
        match x {
            Ref::Sig(k) if r.clocked && r.next.contains_key(&k) => add(Block::Next(k)),
            x => {
                src(x);
            }
        }
        x
    };
    for op in ops {
        op.map(&src, &dst, &|x| x);
    }
    (seen.into_inner().into_iter())
        .map(|x| match x {
            Block::Cur(k) => (x, r.cur[&k], codegen::words(klen(k))),
            Block::Next(k) => (x, r.next[&k], codegen::words(klen(k))),
            Block::Locals => (x, r.base, (b.locs + extra).max(1)),
            Block::Edge => (x, r.edge, 1),
            Block::Oth => (x, r.oth, 2),
        })
        .collect()
}

/// What makes the code of member `b` what it is, but for where its words lie: kind, slots, state
/// variables, the operations with each kernel signal by its order of first use, and which of
/// those have next-state words.
fn shape_key(b: &lower::Body, next: &BTreeMap<u32, u32>) -> ShapeKey {
    let order = std::cell::RefCell::new(Vec::<u32>::new());
    let ord = |r: Ref| match r {
        Ref::Sig(k) => {
            let mut o = order.borrow_mut();
            let i = o.iter().position(|&x| x == k).unwrap_or_else(|| {
                o.push(k);
                o.len() - 1
            });
            Ref::Sig(i as u32)
        }
        r => r,
    };
    let ops: Vec<Op> = (b.ops.iter().chain(&b.init_ops))
        .map(|op| op.map(&ord, &ord, &|x| x))
        .collect();
    let sigs: Vec<bool> = (order.borrow().iter())
        .map(|k| next.contains_key(k))
        .collect();
    (b.kind, b.locs, b.vars.clone(), ops, sigs)
}

type ShapeKey = (Kind, u32, Vec<(u32, u32)>, Vec<Op>, Vec<bool>);

/// The operations of one member with every reference an image slot: a member's slots from
/// `base`, a signal its current words, except the next-state words where a clocked member writes
/// it; the edge word; check sites from `sites`.
#[derive(Clone, Copy)]
struct Resolve<'a> {
    base: u32,
    sites: u32,
    clocked: bool,
    edge: u32,
    oth: u32,
    /// `Op::Other` reads the other image (false: its own, in a loop group).
    rail: bool,
    cur: &'a BTreeMap<u32, u32>,
    next: &'a BTreeMap<u32, u32>,
    /// Signals read from a copy made at the piece's start (a loop group's piece reading what it
    /// drives), and the copy's words.
    snap: &'a BTreeMap<u32, u32>,
}

impl Resolve<'_> {
    fn src(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) => Ref::Loc(*self.snap.get(&k).unwrap_or(&self.cur[&k])),
            r => self.dst(r),
        }
    }

    fn dst(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) if self.clocked && self.next.contains_key(&k) => Ref::Loc(self.next[&k]),
            Ref::Sig(k) => Ref::Loc(self.cur[&k]),
            Ref::Loc(s) => Ref::Loc(self.base + s),
            Ref::Edge => Ref::Loc(self.edge),
            Ref::Oth => Ref::Loc(self.oth),
        }
    }

    fn op(&self, op: &Op) -> Op {
        let o = op.map(&|r| self.src(r), &|r| self.dst(r), &|x| self.sites + x);
        if self.rail { o } else { own_only(o) }
    }
}

fn has_seed(ops: &[Op]) -> bool {
    ops.iter().any(|op| match op {
        Op::Seed { .. } => true,
        Op::If { then, else_, .. } => has_seed(then) || has_seed(else_),
        _ => false,
    })
}

/// `op` with every `Op::Other` reading its own image.
fn own_only(op: Op) -> Op {
    match op {
        Op::Other { dst, src, len, .. } => Op::Copy { dst, src, len },
        Op::If { cond, then, else_ } => Op::If {
            cond,
            then: then.into_iter().map(own_only).collect(),
            else_: else_.into_iter().map(own_only).collect(),
        },
        op => op,
    }
}

#[cfg(test)]
mod tests {
    use super::codes;

    /// The string literals of a source file, outside `//` comments (continued lines joined).
    fn literals(src: &str) -> Vec<String> {
        let (mut out, mut cur, mut chars) = (Vec::new(), None::<String>, src.chars().peekable());
        while let Some(c) = chars.next() {
            match (&mut cur, c) {
                (None, '/') if chars.peek() == Some(&'/') => {
                    chars.by_ref().take_while(|&c| c != '\n').for_each(drop);
                }
                (None, '\'') => {
                    // A char literal (`'"'`) or a lifetime: skip to its end.
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    chars.next();
                }
                (None, '"') => cur = Some(String::new()),
                (Some(_), '"') => out.extend(cur.take()),
                (Some(t), '\\') => match chars.next() {
                    Some('\n') => {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some(e) => t.push(e),
                    None => {}
                },
                (Some(t), c) => t.push(c),
                (None, _) => {}
            }
        }
        out
    }

    /// A40: every reason has a code of its own with a title, why and fix, and no text a user
    /// sees hedges or names an internal rule.
    #[test]
    fn a_colon_in_quoted_code_does_not_split_the_headline() {
        assert_eq!(
            super::construct("a name `<<signal .t.x : bit>>`: it is here"),
            ("a name `<<signal .t.x : bit>>`", "it is here")
        );
        assert_eq!(super::construct("a latch"), ("a latch", ""));
    }

    #[test]
    fn reasons_say_plainly_what_is_not_supported() {
        let mut seen = std::collections::BTreeSet::new();
        for (code, title, why, fix, _) in codes::REASONS {
            assert!(seen.insert(code), "{code} twice");
            assert!(
                !title.is_empty() && !why.is_empty() && !fix.is_empty(),
                "{code}"
            );
        }
        let banned = [
            "the analysis",
            "the lowering",
            "subset checker",
            "cross_delta",
            "run-count",
            "meta_rule",
            "pulse rule",
            "did not count",
            "could not",
            "may ",
            "the normal mode",
            // The headline adds it.
            "not supported in cycle mode",
        ];
        let texts: Vec<String> = (codes::REASONS.iter())
            .flat_map(|r| [r.1, r.2, r.3].map(String::from))
            .chain(literals(include_str!("check.rs")))
            .chain(literals(include_str!("region.rs")))
            .chain(literals(include_str!("lower.rs")))
            .collect();
        for t in &texts {
            for b in banned {
                assert!(!t.contains(b), "{b:?} in {t:?}");
            }
        }
    }
}
