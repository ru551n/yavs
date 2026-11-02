//! `yavs elab` and `yavs run` (docs/cli-contract.md section 5).
//!
//! The design is re-analyzed from source: the closure of TOP is read from the library
//! manifests (each under a shared lock held only for the read, rule 14), every file is
//! analyzed with its recorded standard against one `std`/`ieee` revision (the newest in the
//! closure, or `--std`), then elaborated.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Args as ClapArgs;
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};
use yavs_lib::{Libraries, UnitKind, UnitRef};
use yavs_sem::{Db, LangRev, Options};

use std::io::{Read, Write};

use yavs_kernel::{EndReason, RunStatus};

use crate::StdArg;
use crate::analyze::Out;

#[cfg_attr(test, derive(Debug))]
#[derive(ClapArgs)]
pub struct Common {
    /// Design revision of std/ieee; default: the newest standard in the design's closure.
    #[arg(long, value_enum)]
    std: Option<StdArg>,
    /// The library holding the top level, `NAME:PATH`.
    #[arg(long, value_name = "NAME:PATH", required = true)]
    work: String,
    /// Another library, `NAME:PATH` (repeatable).
    #[arg(long = "map", value_name = "NAME:PATH")]
    maps: Vec<String>,
    /// Turn some errors into warnings: a shared variable of a non-protected type, a pure function
    /// that calls an impure one, an `others` aggregate for an unconstrained target, a non-static
    /// actual of a signal parameter.
    #[arg(long)]
    relaxed: bool,
    /// Read `-- psl` comments as PSL pragmas (native PSL statements are always read).
    #[arg(long)]
    psl: bool,
    /// Define a conditional analysis identifier, `NAME=VALUE`: the same definitions as when the
    /// files were analyzed, because the sources are read again.
    #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = crate::parse_define)]
    defines: Vec<(String, String)>,
    /// Warning control: `-Werror` (every warning is an error), `-Werror=CODE`, `-Wno-CODE` (do not
    /// print it), `-WCODE` (stays a warning under `-Werror`).
    #[arg(short = 'W', value_name = "FLAG")]
    warn: Vec<String>,
    /// Load a shared library for foreign subprograms (E0106 when it cannot be loaded, E0107 when a
    /// subprogram cannot be bound). Also loaded for `elab`, because a signal's initial value may call it. `:args=LAYOUT` says how its functions take their arguments:
    /// `length` (default) or `fat-pointer` (E0136 for any other).
    #[arg(long, value_name = "LIB[:args=LAYOUT]")]
    load: Vec<String>,
    /// Neither read nor write the compilation cache of the work library.
    #[arg(long)]
    no_cache: bool,
    /// Directory of the std/ieee units shared by all libraries, or `off` (default: `$YAVS_STD_CACHE`,
    /// else `yavs/std` in the user's cache directory).
    #[arg(long, value_name = "DIR|off")]
    std_cache: Option<std::ffi::OsString>,
    /// What to do with the `-- yavs coverage exclude` directives in the sources: `apply` (check
    /// them and waive the combinations they name) or `ignore` (read none of them; a note says how
    /// many were left out).
    #[arg(long, value_enum, value_name = "MODE", default_value = "apply")]
    coverage_directives: DirectivesArg,
    /// Keep every `in` port its own signal instead of an alias of its actual (to compare against
    /// the default).
    #[arg(long)]
    no_collapse_ports: bool,
    /// Print the elaborated hierarchy (for debugging yavs).
    #[arg(long, hide = true)]
    print_hierarchy: bool,
    /// `ENTITY`, `ENTITY.ARCH` or `CONFIGURATION` in the work library.
    #[arg(value_name = "TOP")]
    top: String,
    /// A generic of the top level, `-gNAME=VALUE` (the value is taken verbatim).
    #[arg(short = 'g', value_name = "NAME=VALUE", value_parser = crate::parse_define)]
    generics: Vec<(String, String)>,
}

/// What `--coverage-directives` takes.
#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DirectivesArg {
    /// Check the directives and waive what they name.
    Apply,
    /// Read none of them.
    Ignore,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum OnOffArg {
    On,
    Off,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, clap::ValueEnum)]
enum ResolutionArg {
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    Sec,
}

#[cfg_attr(test, derive(Debug))]
#[derive(ClapArgs)]
pub struct RunArgs {
    #[command(flatten)]
    common: Common,
    /// A report at or above this severity stops the simulation with exit 1. Without the
    /// option the default is `failure`, and an `error` report is printed, the run continues
    /// and exits 1 at the end; given explicitly, a report below the level never fails the run.
    #[arg(long, value_enum)]
    exit_severity: Option<SeverityArg>,
    /// `off` suppresses the warnings raised inside `ieee` packages, such as `NUMERIC_STD.TO_INTEGER:
    /// metavalue detected` (default `on`).
    #[arg(long, value_name = "on|off", value_enum)]
    ieee_warnings: Option<OnOffArg>,
    /// Write an FST waveform file of the whole design (an unwritable path is E0801).
    #[arg(long, value_name = "FILE.fst")]
    wave: Option<String>,
    /// Record only signals whose path (`top.u1.sig`) matches this glob (repeatable).
    #[arg(long, value_name = "GLOB")]
    wave_include: Vec<String>,
    /// Do not record signals or scopes whose path matches this glob (repeatable).
    #[arg(long, value_name = "GLOB")]
    wave_exclude: Vec<String>,
    /// Leave out arrays and vectors with more elements than this.
    #[arg(long, value_name = "N", default_value = "65536")]
    wave_max_array: usize,
    /// With `--wave`: also record the variables of processes and shared variables (sampled at the
    /// end of each time step); the include and exclude globs apply to `top.proc.var`. A debug
    /// session (`--dap`, `--interactive`, `--do`) that records a wave does this on its own.
    #[arg(long, requires = "wave")]
    wave_variables: bool,
    /// Print the wall and CPU time of each phase (analyze, elaborate, init, simulate) to
    /// standard error when the simulation ends.
    #[arg(long)]
    time_report: bool,
    /// Sample where the simulation spends its time and print the busiest processes, instances
    /// and subprograms to standard error when the run ends; with FILE also write them as
    /// JSON.
    #[arg(long, value_name = "FILE", num_args = 0..=1, require_equals = true)]
    profile: Option<Option<String>>,
    /// Count process activations and signal events and print the busiest to standard error
    /// when the run ends; with FILE also write them as JSON. Costs run time only when given.
    #[arg(long, value_name = "FILE", num_args = 0..=1, require_equals = true)]
    activity: Option<Option<String>>,
    /// Print the counts of assertion failures and reports by severity and source place to
    /// standard error when the run ends; with `--message-format=json`, one JSON object.
    #[arg(long)]
    assert_summary: bool,
    /// Stop the simulation at this time and exit 0: a number and a unit, `10ms` or `10 ns`
    /// (a bare number is E0105).
    #[arg(long, value_name = "TIME")]
    stop_time: Option<String>,
    /// Print what every waiting process waits for when the run ends at `--stop-time` without
    /// `stop` or `finish`, or runs out of events with a process still waiting for a signal; at
    /// most MAX processes are listed per part (default 30). `kill -USR1 PID` prints the same
    /// report of a running simulation (Unix, always available).
    #[arg(long, value_name = "MAX", num_args = 0..=1, require_equals = true, default_missing_value = "30")]
    hang_report: Option<usize>,
    /// Most delta cycles at one simulated time before the run fails with E3002.
    #[arg(long, value_name = "N", default_value = "10000", value_parser = parse_max_deltas)]
    max_deltas: u32,
    /// Time resolution limit (`std.env.resolution_limit`, TIME values in reflection).
    #[arg(long, value_enum, default_value = "fs")]
    resolution: ResolutionArg,
    /// Worker threads for process evaluation (at least 1). The output does not depend on it.
    #[arg(long, value_name = "N", default_value = "1", value_parser = parse_threads)]
    threads: usize,
    /// Collect the heap of access values once this much was allocated since the last
    /// collection (and as much as that collection kept): plain bytes or a `k`, `m` or `g`
    /// suffix (powers of 1024), at least 1. Default 1m.
    #[arg(long, value_name = "SIZE", value_parser = parse_size)]
    gc_threshold: Option<u64>,
    /// Run the processes of a delta cycle in a random order, to find testbenches that depend on
    /// the order. SEED defaults to 1; the seed is printed at the start so that a failing run can
    /// be replayed with `--shuffle=SEED`.
    #[arg(long, value_name = "SEED", num_args = 0..=1, require_equals = true, default_missing_value = "1")]
    shuffle: Option<u64>,
    /// Count coverage and write the run's coverage database (`--cov-file`). LETTERS selects the
    /// kinds, in any order: `s` statement, `b` branch, `c` condition, `e` expression, `t` toggle,
    /// `f` fsm, `a` assert, `m` combination; for example `--cov=sbt`. No kind is on by default.
    #[arg(long, value_name = "LETTERS", num_args = 0..=1, require_equals = true, default_missing_value = "")]
    cov: Option<String>,
    /// Count coverage of these kinds, comma separated (`statement`, `branch`, `toggle`, ...): the
    /// long form of `--cov=LETTERS`, not with it.
    #[arg(long, value_enum, value_delimiter = ',', value_name = "KIND,...")]
    cov_kind: Vec<CovKindArg>,
    /// With coverage on (`--cov` or `--cov-kind`): the coverage database to write (default
    /// `yavs.ycov`).
    #[arg(long, value_name = "FILE")]
    cov_file: Option<String>,
    /// With coverage on: the name of this run's test in the database (default: the enabled test
    /// case of a `runner_cfg` generic, else the top level with its `-g` generics).
    #[arg(long, value_name = "NAME")]
    cov_test: Option<String>,
    /// With coverage on: leave signals with more `bit`/`std_ulogic` scalars than this out of
    /// toggle coverage.
    #[arg(long, value_name = "N", default_value = "65536")]
    cov_toggle_max: u64,
    /// With coverage on: also leave `translate_off` ... `translate_on` regions out of the coverage.
    #[arg(long)]
    cov_exclude_translate_off: bool,
    /// With coverage on: store the text of the source files in the database, so `yavs cov report
    /// --html` needs no checkout. The source text then travels with the database.
    #[arg(long)]
    cov_embed_sources: bool,
    /// Load a VHPI plugin, `LIB[:ENTRY]` (E0901 when it cannot be loaded, E0902 when the entry
    /// point is missing).
    #[arg(long, value_name = "LIB[:ENTRY]")]
    vhpi: Vec<String>,
    // The text names the tiers this build has, so it is built by `tier_help`.
    #[arg(long, value_enum, help = tier_help())]
    tier: Option<TierArg>,
    /// With a compiled tier: functions of more than N IR instructions stay
    /// interpreted (compile time grows faster than their size).
    #[arg(long, value_name = "N", default_value = "50000")]
    tier_max_size: usize,
    /// With `--tier=auto`: compile a function once the sampler (one sample every PERIOD
    /// microseconds) has seen the interpreter in it for BASE microseconds plus PER_INSTR
    /// nanoseconds per IR instruction it has; PERIOD 0: compile every function the interpreter
    /// enters (for tests and tuning).
    #[arg(long, value_name = "BASE,PER_INSTR,PERIOD", value_parser = heat, hide = true)]
    tier_heat: Option<[u32; 3]>,
    /// Print notes on how the run is set up (functions a compiled tier leaves interpreted or
    /// compiles).
    #[arg(long)]
    verbose: bool,
    /// Run under a Debug Adapter Protocol client: on 127.0.0.1:PORT (0 picks a free port) or
    /// on standard input and output (`stdio`). A port that cannot be opened is E0701.
    #[arg(long, value_name = "PORT|stdio", value_parser = yavs_dap::parse_transport)]
    dap: Option<yavs_dap::Transport>,
    /// End the run (exit status 130) when standard input is closed or ends, for a parent that
    /// holds a pipe to it and may die without a word (SIGKILL, a crash): the run does not outlive
    /// it, even while it waits for a debugger. Takes standard input for itself (the design and
    /// every library see the null device), so not with `--interactive`; with `--dap=stdio` the
    /// protocol stream plays this part. Waveform and coverage files are not finished.
    #[arg(long, conflicts_with = "interactive")]
    exit_on_stdin_close: bool,
    /// Run from a `yavs>` prompt: stops at time 0, then takes console commands (`help` lists
    /// them; run, step, examine, force, breakpoints, restart, source, quit).
    #[arg(long, conflicts_with = "dap")]
    interactive: bool,
    /// Run these console commands, separated by `;`. Repeatable, in order. Without
    /// `--interactive` the run then exits with its status (1 at a command that fails).
    #[arg(long = "do", value_name = "COMMANDS", conflicts_with = "dap")]
    do_commands: Vec<String>,
    /// Show the waveform in Surfer (0.6.0 or later) while the run writes it: start `surfer` from
    /// PATH, or with `=HOST:PORT` connect to a running one whose WCP server listens there.
    /// Without `--wave`, records `<top>.fst` in the directory that holds the work library.
    #[arg(long, value_name = "HOST:PORT", num_args = 0..=1, require_equals = true)]
    surfer: Option<Option<String>>,
    /// How often the waveform is written out and Surfer reloads it (default `1s`; `500ms`,
    /// `10s`); wins over `YAVS_WAVE_FLUSH`.
    #[arg(long, value_name = "DURATION", requires = "surfer", value_parser = parse_refresh)]
    surfer_refresh: Option<std::time::Duration>,
    /// `--wave` was not given and `--surfer` chose the file.
    #[arg(skip)]
    wave_default: bool,
    /// In a debug session (`--dap`, `--interactive`, `--do`), show the source file COMPILED as
    /// ORIGINAL and take breakpoints in ORIGINAL: for files compiled from a rewritten copy with
    /// the same lines. Repeatable.
    #[arg(long, value_name = "COMPILED=ORIGINAL", value_parser = crate::parse_define)]
    source_map: Vec<(String, String)>,
    /// Plusargs `+NAME[=VALUE]` after the top level, seen by VHPI plugins as tool arguments.
    #[arg(value_name = "+NAME[=VALUE]", value_parser = plusarg)]
    plusargs: Vec<String>,
}

/// The compiled tiers of this build, for sentences like "this yavs has ...".
pub(crate) fn tiers_built() -> &'static str {
    #[cfg(test)]
    if crate::reference::GENERIC_TIERS.get() {
        return "the tiers `yavs --version` names";
    }
    match (cfg!(feature = "llvm"), cfg!(feature = "clift")) {
        (true, true) => "the Cranelift and LLVM tiers",
        (false, true) => "the Cranelift tier",
        (true, false) => "the LLVM tier",
        (false, false) => "no compiled tier, only `interp`",
    }
}

/// The help of `--tier`: what the tiers are and which of them this build has.
fn tier_help() -> String {
    let built = tiers_built();
    format!(
        "Execution tier: `interp` runs the interpreter alone; `auto` runs it and compiles the \
         functions that become hot on a background thread; `clift` and `llvm` compile every \
         function before time 0; `clift-auto` is `auto` with Cranelift; `auto3` has Cranelift \
         compile warm functions and LLVM O2 those that stay hot. A tier needs its code generator \
         in the build (error E0120 otherwise): this yavs has {built}. Default: `auto` with a compiled tier, `interp` with \
         `--dap`, `--interactive` or `--do`, and without one."
    )
}

/// `--surfer-refresh`: a duration above zero.
fn parse_refresh(s: &str) -> Result<std::time::Duration, String> {
    parse_flush_interval(s)
        .filter(|d| !d.is_zero())
        .ok_or_else(|| format!("`{s}`: use a duration above zero such as `1s` or `500ms`"))
}

/// `YAVS_WAVE_FLUSH` duration: whole `s` or `ms` (`5s`, `500ms`).
fn parse_flush_interval(s: &str) -> Option<std::time::Duration> {
    let (n, unit): (&str, fn(u64) -> std::time::Duration) = match s.strip_suffix("ms") {
        Some(n) => (n, std::time::Duration::from_millis),
        None => (s.strip_suffix('s')?, std::time::Duration::from_secs),
    };
    n.parse().ok().map(unit)
}

/// `4194304`, `4m`, `64K`, `1g`: a byte count of at least 1.
fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (digits, mul) = match t.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (
            &t[..i],
            match c.to_ascii_lowercase() {
                'k' => 1u64 << 10,
                'm' => 1 << 20,
                'g' => 1 << 30,
                _ => return Err(format!("`{s}`: unknown suffix `{c}`, expected k, m or g")),
            },
        ),
        _ => (t, 1),
    };
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("`{s}` is not a size: expected bytes with an optional k, m or g"))?;
    match n.checked_mul(mul) {
        Some(0) => Err("the threshold must be at least 1 byte".into()),
        Some(b) => Ok(b),
        None => Err(format!("`{s}` is too large")),
    }
}

fn parse_threads(s: &str) -> Result<usize, String> {
    match s.trim().parse::<usize>() {
        Ok(0) => Err("the number of threads must be at least 1".into()),
        Ok(n) => Ok(n),
        Err(_) if s.trim() == "auto" => Err(
            "`auto` is not implemented yet (#901): give a number of threads, such as `--threads=4`"
                .into(),
        ),
        Err(_) => Err(format!(
            "`{s}` is not a number of threads: give a whole number of at least 1, such as `--threads=4`"
        )),
    }
}

fn parse_max_deltas(s: &str) -> Result<u32, String> {
    match s.trim().parse::<u32>() {
        Ok(0) => Err("the limit must be at least 1".into()),
        Ok(n) => Ok(n),
        Err(_) => Err(format!(
            "`{s}` is not a number of delta cycles: give a whole number of at least 1"
        )),
    }
}

fn heat(s: &str) -> Result<[u32; 3], String> {
    let v: Vec<u32> = s
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<u32>()
                .map_err(|_| format!("`{v}` is not a whole number"))
        })
        .collect::<Result<_, _>>()?;
    v.try_into()
        .map_err(|_| format!("expected BASE,PER_INSTR,PERIOD, found `{s}`"))
}

fn plusarg(s: &str) -> Result<String, String> {
    if s.starts_with('+') && s.len() > 1 {
        Ok(s.to_string())
    } else {
        Err(format!("expected +NAME or +NAME=VALUE, found `{s}`"))
    }
}

/// The letters of `--cov=LETTERS`, with the kind each selects.
const COV_LETTERS: [(char, CovKindArg); 8] = [
    ('s', CovKindArg::Statement),
    ('b', CovKindArg::Branch),
    ('c', CovKindArg::Condition),
    ('e', CovKindArg::Expression),
    ('t', CovKindArg::Toggle),
    ('f', CovKindArg::Fsm),
    ('a', CovKindArg::Assert),
    ('m', CovKindArg::Combination),
];

impl CovKindArg {
    fn word(self) -> String {
        use clap::ValueEnum;
        let v = self.to_possible_value();
        v.map(|v| v.get_name().to_string()).unwrap_or_default()
    }
}

/// The table of coverage kinds, letter and word, for the notes of E0140.
fn cov_letter_table() -> String {
    let rows: Vec<String> = COV_LETTERS
        .iter()
        .map(|(l, k)| format!("  {l}  {}", k.word()))
        .collect();
    format!(
        "the kinds, as the letters of `--cov=LETTERS` or the words of `--cov-kind=LIST`:\n{}",
        rows.join("\n")
    )
}

impl RunArgs {
    /// Turns `--cov=LETTERS`, `--cov-kind` and `--cov-file` into the kinds to count (`cov_kind`)
    /// and the database path (`cov`), before the run starts. No kind is on by default: coverage
    /// on without a kind, a letter that is none, or `--cov-file` and the like with coverage off
    /// are E0140.
    fn resolve_cov(&mut self) -> Result<(), Box<Diagnostic>> {
        let err = |m: String| Diagnostic::error("E0140", m);
        let example = "for example `--cov=sbt`, or `--cov-kind=statement,branch,toggle`";
        let kinds = match (self.cov.take(), self.cov_kind.is_empty()) {
            (Some(_), false) => {
                return Err(Box::new(
                    err("`--cov` and `--cov-kind` both select the coverage kinds".into())
                        .with_help(
                            "give the kinds once: `--cov=LETTERS`, or `--cov-kind=LIST` with no `--cov`",
                        ),
                ));
            }
            (Some(l), true) if l.is_empty() => {
                return Err(Box::new(
                    err("coverage was asked for, but no kind was selected".into())
                        .with_note("no kind is counted by default")
                        .with_note(cov_letter_table())
                        .with_help(example),
                ));
            }
            (Some(l), true) => {
                let mut kinds = Vec::new();
                for c in l.chars() {
                    let Some((_, k)) = COV_LETTERS.iter().find(|(x, _)| *x == c) else {
                        let words: Vec<String> =
                            COV_LETTERS.iter().map(|(_, k)| k.word()).collect();
                        let close = if words.contains(&l) {
                            Some(format!("`{l}` is a coverage kind"))
                        } else {
                            let cands = words.iter().map(|w| (w.as_str(), w.as_str()));
                            yavs_diag::suggest::suggest(&l, "kind", cands)
                        };
                        let mut d = err(format!(
                            "`{c}` in `--cov={l}` is not a coverage kind letter"
                        ))
                        .with_note(cov_letter_table())
                        .with_help(
                            "the coverage database file goes in `--cov-file=FILE`, not in `--cov`",
                        );
                        if let Some(h) = close {
                            d = d.with_help(format!("{h}; words go in `--cov-kind=LIST`"));
                        }
                        return Err(Box::new(d.with_help(example)));
                    };
                    kinds.push(*k);
                }
                kinds
            }
            (None, false) => std::mem::take(&mut self.cov_kind),
            (None, true) => {
                let off = |what: &str| {
                    Box::new(
                        err(format!("{what} needs coverage on, and none is"))
                            .with_help(format!("add `--cov=LETTERS` ({example})")),
                    )
                };
                if self.cov_file.is_some() {
                    return Err(off("`--cov-file`"));
                }
                if self.cov_embed_sources {
                    return Err(off("`--cov-embed-sources`"));
                }
                return Ok(());
            }
        };
        self.cov = Some(self.cov_file.take().unwrap_or_else(|| "yavs.ycov".into()));
        self.cov_kind = kinds;
        Ok(())
    }

    /// Whether the run goes through the debug server: a client or the console drives it.
    fn debugged(&self) -> bool {
        self.dap.is_some() || self.interactive || !self.do_commands.is_empty()
    }

    /// Whether the wave file holds variables: asked for, or a debug session, where `wave add` of a
    /// variable then has its history since time 0.
    fn wave_vars(&self) -> bool {
        self.wave.is_some() && (self.wave_variables || self.debugged())
    }

    /// Why `vhpi_control(vhpiReset)` cannot work in this run (#832): these keep what happened so
    /// far outside the simulation.
    fn no_vhpi_reset(&self) -> Option<String> {
        let option = if self.debugged() {
            "a debug session"
        } else if self.wave.is_some() {
            "`--wave`"
        } else if self.cov.is_some() {
            "`--cov`"
        } else if self.profile.is_some() {
            "`--profile`"
        } else if self.activity.is_some() {
            "`--activity`"
        } else if self.assert_summary {
            "`--assert-summary`"
        } else {
            return None;
        };
        Some(format!(
            "not with {option}, which reports on the whole run: run without it"
        ))
    }

    /// The tier of this run: the one asked for, else `auto` where it is built in and nothing
    /// needs the interpreter alone (a debugger reads frame slots).
    pub(crate) fn tier(&self) -> TierArg {
        let compiled = cfg!(feature = "llvm") || cfg!(feature = "clift");
        let t = self.tier.unwrap_or(if compiled && !self.debugged() {
            TierArg::Auto
        } else {
            TierArg::Interp
        });
        // Without LLVM, `auto` is the same policy with Cranelift (user, 2026-10-24).
        if t == TierArg::Auto && !cfg!(feature = "llvm") && cfg!(feature = "clift") {
            TierArg::CliftAuto
        } else {
            t
        }
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TierArg {
    /// The interpreter alone.
    Interp,
    /// LLVM: every function before time 0 (the `llvm` feature).
    Llvm,
    /// The interpreter, and the functions that become hot compiled on a background thread.
    Auto,
    /// Cranelift: every function before time 0 (the `clift` feature).
    Clift,
    /// Cranelift: the functions that become hot, on a background thread.
    CliftAuto,
    /// Three tiers: the interpreter, Cranelift for warm functions, LLVM O2 for those that stay
    /// hot (needs the `llvm` and `clift` features).
    Auto3,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CovKindArg {
    /// Each statement executed.
    Statement,
    /// Each arm of `if`, `case`, conditional and selected assignments, and loop bodies.
    Branch,
    /// The 0 to 1 and 1 to 0 changes of every `bit` and `std_ulogic` scalar of signals and ports.
    Toggle,
    /// Each input and value of conditions with two or more inputs, `wait until` included.
    Condition,
    /// The same for the conditions of `assert`. A row that fails the assertion is hit only by a
    /// failing run, so these rows are asked for apart; they report as conditions.
    Assert,
    /// The same for the logic expression assigned to a signal or variable.
    Expression,
    /// Each combination of the inputs of conditions and logic expressions that can occur
    /// (an input a short-circuit operator skips counts as not evaluated, not as false).
    Combination,
    /// Each state a state machine reached and each transition it took, per instance.
    Fsm,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum SeverityArg {
    Note,
    Warning,
    Error,
    Failure,
}

/// The elaborated design, or the exit code of the failed stage.
fn elaborate(
    c: &Common,
    lowering: yavs_elab::LowerOptions,
    compile_all: bool,
    collapse_ports: bool,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> Result<(Out, Db, yavs_elab::Elaborated), (Out, u8)> {
    let mut out = Out::new(format, color);
    let none = SourceMap::default();
    let mut libs = Libraries::new();
    for spec in std::iter::once(&c.work).chain(&c.maps) {
        let name = spec
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let r = if name == "std" || name == "ieee" {
            Err(Box::new(
                Diagnostic::error("E0102", format!("library `{name}` is provided by yavs"))
                    .with_help("remove the `std`/`ieee` mapping; yavs resolves them itself"),
            ))
        } else {
            libs.add_spec(spec)
        };
        if let Err(d) = r {
            out.emit(&none, &d);
            return Err((out, 2));
        }
    }
    if let Err(d) = out.set_warn(&c.warn) {
        out.emit(&none, &d);
        return Err((out, 2));
    }
    let (work, _) = c.work.split_once(':').expect("checked by add_spec");
    let work = yavs_lib::fold(work);
    // The closure of TOP: an architecture, an entity (with its latest architecture) or a
    // configuration.
    let refs: Vec<UnitRef> = match c.top.split_once('.') {
        Some((e, a)) => vec![UnitRef::new(
            &work,
            UnitKind::Architecture,
            &format!("{}/{}", yavs_lib::fold(e), yavs_lib::fold(a)),
        )],
        None => vec![
            UnitRef::new(&work, UnitKind::Entity, &c.top),
            UnitRef::new(&work, UnitKind::Configuration, &c.top),
        ],
    };
    let mut files = Err(None);
    // A top level that is not in the library: `L0031` for the top itself, without the "needed by"
    // note of a missing dependency.
    let miss = |d: &Diagnostic| d.code == "L0031" && d.notes.is_empty();
    for r in &refs {
        match yavs_lib::closure(&libs, r) {
            Ok(f) => {
                files = Ok(f);
                break;
            }
            Err(d) => match &files {
                Err(None) => files = Err(Some(d)),
                // The first miss stays; a real failure (a changed or unreadable source file, a
                // cycle, a missing dependency) is never reported in place of it or hidden by a
                // later miss.
                Err(Some(p)) if miss(p) && !miss(&d) => files = Err(Some(d)),
                Err(Some(_)) => {}
                Ok(_) => {}
            },
        }
    }
    let files = match files {
        Ok(f) => f,
        Err(d) => {
            let d = match d {
                Some(d) if miss(&d) => top_not_found(&c.top, &work, *d),
                Some(d) => d,
                None => Box::new(Diagnostic::error("E0104", "no top level")),
            };
            out.emit(&none, &d);
            return Err((out, 2));
        }
    };
    let rev = match c.std {
        Some(StdArg::V1993) => LangRev::Vhdl1993,
        Some(StdArg::V2008) => LangRev::Vhdl2008,
        Some(StdArg::V2019) => LangRev::Vhdl2019,
        // The newest standard recorded in the closure (cli-contract section 2).
        None if files.iter().any(|f| f.std == "2019") => LangRev::Vhdl2019,
        None if files.iter().any(|f| f.std == "2008") => LangRev::Vhdl2008,
        None if !files.is_empty() => LangRev::Vhdl1993,
        None => LangRev::Vhdl2008,
    };
    let mut opts = Options::new(rev, yavs_sem::default_stdlib());
    opts.libraries = libs;
    opts.relaxed = c.relaxed;
    opts.cov_directives = lowering.cov_directives;
    opts.psl = c.psl;
    opts.defines = c.defines.clone();
    opts.encoding = encoding;
    let work_dir = c.work.split_once(':').map_or("", |(_, d)| d);
    let shared = match crate::cache::shared_std_dir(c.std_cache.as_deref()) {
        Ok(d) => d,
        Err(d) if !c.no_cache => {
            out.emit(&none, &d);
            return Err((out, 2));
        }
        Err(_) => None,
    };
    let (mut db, analyzed) = crate::cache::analyze_closure(
        opts,
        &files,
        work_dir.as_ref(),
        shared.as_deref(),
        !c.no_cache,
        !compile_all,
    );
    let diags = db.take_diagnostics();
    for d in &diags {
        out.emit(&db.sources, d);
    }
    if analyzed.is_err() || out.errors > 0 {
        return Err((out, 2));
    }
    coverage_directives(&mut out, &db, &lowering);
    if out.errors > 0 {
        return Err((out, 2));
    }
    crate::timing::mark("analyze");
    let r = yavs_elab::find_top(&mut db, &work, &c.top).and_then(|t| {
        let cfg = yavs_elab::ElabConfig {
            collapse_ports: collapse_ports && !c.no_collapse_ports,
            // Hidden test aid: `YAVS_PACKS=off` gives every leaf a kernel signal of its own.
            packs: std::env::var("YAVS_PACKS").map_or(true, |v| v != "off"),
            ..yavs_elab::ElabConfig::default()
        };
        yavs_elab::elaborate(&mut db, t, &c.generics, &cfg)
    });
    match r {
        Ok(mut e) => {
            for d in &e.warnings {
                out.emit(&db.sources, d);
            }
            if let Err(ds) = e.lower_with(&mut db, lowering) {
                for d in &ds {
                    out.emit(&db.sources, d);
                }
                return Err((out, 2));
            }
            if out.errors > 0 {
                return Err((out, 2));
            }
            crate::timing::mark("elaborate");
            Ok((out, db, e))
        }
        Err(ds) => {
            for d in &ds {
                out.emit(&db.sources, d);
            }
            Err((out, 2))
        }
    }
}

/// Say that the in-source coverage directives are left out (`--coverage-directives=ignore`), or,
/// when combinations are measured, warn about the decisions that cannot be. Their checks ran with
/// the analysis (`cache.rs`), whether or not coverage is collected.
fn coverage_directives(out: &mut Out, db: &Db, lowering: &yavs_elab::LowerOptions) {
    if !lowering.cov_directives {
        if let n @ 1.. = yavs_elab::covdir::count(db) {
            out.emit(
                &db.sources,
                &Diagnostic::new(
                    yavs_diag::Severity::Note,
                    "",
                    format!(
                        "`--coverage-directives=ignore`: {n} `-- yavs coverage exclude` directive{} left out, none checked or applied",
                        if n == 1 { "" } else { "s" }
                    ),
                ),
            );
        }
    } else if lowering.cov_combination {
        for d in yavs_elab::covdir::unmeasured_all(db) {
            out.emit(&db.sources, &d);
        }
    }
}

/// `E0104`: TOP names no entity, architecture or configuration of the work library. `miss` is
/// the library layer's `L0031` for it, whose help says what the library holds.
fn top_not_found(top: &str, work: &str, miss: Diagnostic) -> Box<Diagnostic> {
    let mut d = Diagnostic::error(
        "E0104",
        format!("top level `{top}` not found in library `{work}`"),
    );
    match top.split_once('.') {
        Some((e, a)) => {
            d = d.with_note(format!(
                "no architecture `{a}` of entity `{e}` in the library"
            ));
            if yavs_lib::fold(e) == work {
                d = d.with_help(format!(
                    "TOP has no library prefix: write `{a}` or `{a}.ARCH` for an entity or \
                     configuration of the work library"
                ));
            }
        }
        None => {
            d = d.with_note("neither an entity nor a configuration of that name is in the library");
        }
    }
    for h in miss.help {
        d = d.with_help(h);
    }
    Box::new(d)
}

/// The library and the argument layout of a `--load` value, `LIB[:args=LAYOUT]`.
fn load_value(value: &str) -> Result<(&str, yavs_vhpi::ArgLayout), Box<Diagnostic>> {
    use yavs_vhpi::ArgLayout;
    Ok(match value.rsplit_once(":args=") {
        None => (value, ArgLayout::Length),
        Some((lib, "length")) => (lib, ArgLayout::Length),
        Some((lib, "fat-pointer")) => (lib, ArgLayout::FatPointer),
        Some((_, other)) => {
            let key = other.to_ascii_lowercase();
            let layouts = ["length", "fat-pointer"].map(|l| (l, l));
            let mut d = Diagnostic::error(
                "E0136",
                format!("unknown argument layout `{other}` in `--load={value}`"),
            );
            if let Some(h) = yavs_sem::suggest::suggest(&key, "layout", layouts.into_iter()) {
                d = d.with_help(h);
            }
            return Err(Box::new(d.with_help(
                "the layouts are `length` (the default) and `fat-pointer`",
            )));
        }
    })
}

/// Loads the `--load` libraries, before elaboration: a signal's initial value may call them.
fn load_libraries(load: &[String]) -> Result<Vec<yavs_vhpi::ForeignLib>, Box<Diagnostic>> {
    let mut libs = Vec::new();
    for l in load {
        let (path, layout) = load_value(l)?;
        let lib = yavs_vhpi::load_foreign(path.as_ref(), layout).map_err(|err| {
            let d = Diagnostic::error("E0106", format!("cannot load library `{path}` (`--load`)"));
            Box::new(match err {
                yavs_vhpi::LoadError::NotFound { tried, .. } => {
                    d.with_note(format!("no such file: {}", tried.display()))
                }
                yavs_vhpi::LoadError::Open { tried, os, .. } => d
                    .with_note(format!("tried {tried}"))
                    .with_note(format!("the OS loader said: {os}"))
                    .with_help(
                        "check that the library matches yavs's architecture and that its own \
                         dependencies are on the library search path",
                    ),
                yavs_vhpi::LoadError::NoEntry { .. } => d,
            })
        })?;
        libs.push(lib);
    }
    Ok(libs)
}

/// Loads the `--load` libraries and makes elaboration-time calls of foreign subprograms use them.
fn install_foreign(load: &[String]) -> Result<Vec<yavs_vhpi::ForeignLib>, Box<Diagnostic>> {
    let libs = load_libraries(load)?;
    if !libs.is_empty() {
        let libs = libs.clone();
        yavs_interp::set_foreign_binder(std::sync::Arc::new(move |x| {
            let f = yavs_vhpi::bind_foreign(&libs, &x.symbol, |l| x.signature(l))?;
            Some(
                f.map_err(|e| e.to_string())
                    .and_then(|f| x.unsupported_in(f.layout()).map_or(Ok(f), Err)),
            )
        }));
    }
    Ok(libs)
}

/// Binds every foreign subprogram of the program to its symbol: in the libraries in order, then
/// in the executable (ir.md 1.7).
fn bind_foreign(
    libs: &[yavs_vhpi::ForeignLib],
    e: &mut yavs_elab::Elaborated,
    out: &mut Out,
) -> Result<(), Box<Diagnostic>> {
    let Some(ir) = e.ir.as_mut() else {
        return Ok(());
    };
    let searched = libs
        .iter()
        .map(|l| format!("`{}`, ", l.path.display()))
        .collect::<String>()
        + "the yavs executable";
    ir.foreign_searched = searched.clone();
    for (id, x) in ir.program.foreign.iter() {
        if let Some(m) = &x.model {
            match yavs_vhpi::foreign_model(&m.library, &m.model, x.ret.is_some()) {
                Ok(Some(h)) => ir.models.push((id, h)),
                Err(err) => {
                    return Err(Box::new(Diagnostic::error(
                        "E0107",
                        format!("foreign subprogram `{}` cannot be called: {err}", x.name),
                    )));
                }
                // Warn early, fail at the call, as for a missing VHPIDIRECT symbol.
                Ok(None) => out.emit(
                    &SourceMap::default(),
                    &Diagnostic::warning(
                        "W0108",
                        format!(
                            "VHPI model `{} {}` of foreign subprogram `{}` is not registered",
                            m.library, m.model, x.name
                        ),
                    )
                    .with_note("searched the plugins loaded with `--vhpi`")
                    .with_help(
                        "register it with `vhpi_register_foreignf` in a plugin given as \
                         `--vhpi=LIB`; calling the subprogram is an error (E0107)",
                    ),
                ),
            }
            continue;
        }
        match yavs_vhpi::bind_foreign(libs, &x.symbol, |layout| x.signature(layout)) {
            Some(Ok(f)) => {
                if let Some(err) = x.unsupported_in(f.layout()) {
                    return Err(Box::new(Diagnostic::error(
                        "E0107",
                        format!("foreign subprogram `{}` cannot be called: {err}", x.name),
                    )));
                }
                ir.foreign.push((id, f));
            }
            Some(Err(err)) => {
                return Err(Box::new(Diagnostic::error(
                    "E0107",
                    format!("foreign subprogram `{}` cannot be called: {err}", x.name),
                )));
            }
            // Warn early, fail at the call (decisions.md "Foreign subprograms and `--load`").
            None => out.emit(
                &SourceMap::default(),
                &Diagnostic::warning(
                    "W0108",
                    format!(
                        "symbol `{}` of foreign subprogram `{}` is not defined",
                        x.symbol, x.name
                    ),
                )
                .with_note(format!("searched {searched}"))
                .with_help(
                    "load the shared library that defines it with `--load=LIB`; calling the \
                     subprogram is an error (E0107)",
                ),
            ),
        }
    }
    Ok(())
}

/// What an object of the hierarchy is, for `--print-hierarchy`: `signal`, `port in`, ...
fn object_kind(k: yavs_elab::ObjKind) -> String {
    use yavs_elab::ObjKind::*;
    let mode = |m: yavs_kernel::PortMode| match m {
        yavs_kernel::PortMode::In => "in",
        yavs_kernel::PortMode::Out => "out",
        yavs_kernel::PortMode::Inout => "inout",
        yavs_kernel::PortMode::Buffer => "buffer",
    };
    match k {
        Signal => "signal".into(),
        Port(m) => format!("port {}", mode(m)),
        ComponentPort(m) => format!("component port {}", mode(m)),
        Implicit => "implicit signal".into(),
    }
}

fn print_hierarchy(db: &Db, e: &yavs_elab::Elaborated) {
    use yavs_elab::Case;
    let what: std::collections::HashMap<_, _> = (e.procs.iter())
        .filter_map(|p| Some((p.scope, p.what.as_ref()?)))
        .collect();
    let mut s = String::new();
    for (i, sc) in e.hier.scopes.iter().enumerate() {
        let id = yavs_elab::ScopeId(i as u32);
        if matches!(sc.kind, yavs_elab::ScopeKind::Component { .. }) {
            continue;
        }
        let unlabelled = what.get(&id);
        match (unlabelled, sc.parent) {
            (Some(what), Some(region)) => {
                let region = e.hier.instance_name(region, db, Case::Original);
                s.push_str(&format!("{}.{what}", yavs_diag::dotted(&region)));
            }
            _ => s.push_str(&yavs_diag::dotted(&e.hier.instance_name(
                id,
                db,
                Case::Original,
            ))),
        }
        s.push('\n');
        for &o in &sc.objs {
            let l = e.nets.obj(o);
            s.push_str(&format!(
                "{}.{} ({})\n",
                e.hier.dotted(id),
                l.name.text,
                object_kind(l.kind)
            ));
        }
    }
    print!("{s}");
}

pub fn elab(
    a: Common,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    // The libraries stay loaded until the process ends (a design's initial values call them).
    let _libs = match install_foreign(&a.load) {
        Ok(libs) => libs,
        Err(d) => {
            let mut out = Out::new(format, color);
            out.emit(&SourceMap::default(), &d);
            return out.finish(2);
        }
    };
    match elaborate(
        &a,
        yavs_elab::LowerOptions::default(),
        false,
        true,
        format,
        color,
        encoding,
    ) {
        Ok((out, db, e)) => {
            if a.print_hierarchy {
                print_hierarchy(&db, &e);
            }
            std::mem::forget(db);
            out.finish(0)
        }
        Err((out, code)) => out.finish(code),
    }
}

/// Ends tier-up when the run is over (`AutoTier::stop`): `end` at the normal end, without
/// waiting for a function still compiling; on any other return the drop waits for it.
enum StopTierUp {
    #[cfg(any(feature = "llvm", feature = "clift"))]
    Llvm(std::sync::Arc<yavs_llvm::AutoTier>),
}

impl StopTierUp {
    /// Exit with `code` at once if the compile thread is still busy, so the run never waits
    /// for compilation; with `keep_exit` (foreign libraries or VHPI plugins loaded, whose exit
    /// handlers must run) wait for it instead.
    #[allow(unused_variables)]
    fn end(self, code: u8, keep_exit: bool) {
        match &self {
            #[cfg(any(feature = "llvm", feature = "clift"))]
            StopTierUp::Llvm(t) => {
                if !t.stop(keep_exit) {
                    yavs_llvm::exit_now(code);
                }
            }
            #[cfg(not(any(feature = "llvm", feature = "clift")))]
            _ => {}
        }
    }

    /// Starts the compile thread. It does not exist at first: a run of a few milliseconds ends
    /// before it would look at anything, and a thread is a twentieth of such a run. The helper
    /// thread of the signal handlers calls this after `START_AFTER` (`Handlers::after`).
    #[allow(unused_variables)]
    fn launcher(
        &self,
        format: MessageFormat,
        color: bool,
    ) -> (std::time::Duration, Arc<dyn Fn() + Send + Sync>) {
        match self {
            #[cfg(any(feature = "llvm", feature = "clift"))]
            StopTierUp::Llvm(t) => {
                let tier = t.clone();
                (
                    yavs_llvm::START_AFTER,
                    Arc::new(move || {
                        if let Err(e) = tier.launch() {
                            crate::stdout::flush();
                            let d = Diagnostic::warning(
                                "W0117",
                                "cannot start the compile thread of `--tier=auto`",
                            )
                            .with_note(e.to_string());
                            eprint!(
                                "{}",
                                yavs_diag::render(&SourceMap::default(), &d, format, color)
                            );
                        }
                    }),
                )
            }
            #[cfg(not(any(feature = "llvm", feature = "clift")))]
            _ => unreachable!("no tier-up without a compiled tier"),
        }
    }
}

impl Drop for StopTierUp {
    fn drop(&mut self) {
        match self {
            #[cfg(any(feature = "llvm", feature = "clift"))]
            StopTierUp::Llvm(t) => {
                t.stop(true);
            }
            #[cfg(not(any(feature = "llvm", feature = "clift")))]
            _ => {}
        }
    }
}

/// The work library's compilation cache keeps the code Cranelift compiles (#481), as the LLVM tier's
/// does (#505); not with `--no-cache`.
#[cfg(feature = "clift")]
fn clift_cache(a: &RunArgs) {
    let work = a.common.work.split_once(':').map_or("", |(_, d)| d);
    if !a.common.no_cache && !work.is_empty() {
        yavs_clift::set_cache(Some(std::path::Path::new(work)));
        // The run-to-run profile of the tier-up policy is of this command line (#754).
        yavs_llvm::set_run_id(Some(&run_id()));
    }
}

/// `--tier=clift` (compile the program now) or `--tier=clift-auto` (start tier-up).
#[cfg(feature = "clift")]
fn use_clift(
    rt: &yavs_interp::Runtime,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    clift_cache(a);
    let bug = |e: String| {
        Box::new(
            Diagnostic::error("E0121", "the Cranelift tier cannot compile the design")
                .with_note(e)
                .with_note("this is a bug in yavs"),
        )
    };
    if a.tier() == TierArg::Clift {
        let mut tier = yavs_clift::compile(&rt.prog, a.tier_max_size).map_err(bug)?;
        tier.post = yavs_clift::PostSet::new(rt.prog.clone(), false);
        if a.verbose {
            for (f, why) in &tier.notes {
                let name = rt.prog.func(*f).map_or("?", |f| f.name.as_str());
                eprintln!("note: `{name}` stays interpreted under --tier=clift: {why}");
            }
            let n: usize = tier.interpreted.iter().map(|k| k.1).sum();
            let kinds: Vec<String> = tier
                .interpreted
                .iter()
                .map(|(k, c)| format!("{k} {c}"))
                .collect();
            eprintln!(
                "note: --tier=clift: {} functions ({} IR instructions) compiled in {} ms, {} instructions compiled, {n} executed by the interpreter ({})",
                tier.compiled(),
                tier.instrs,
                tier.compile_time.as_millis(),
                tier.inline,
                kinds.join(", ")
            );
            let (hits, stored) = yavs_clift::cache_stats();
            eprintln!(
                "note: --tier=clift: {hits} functions linked from the compilation cache, {stored} compiled and stored"
            );
            // Per function size class: count, mean and slowest translation time.
            for (lo, hi) in [(0, 64), (64, 256), (256, 1024), (1024, usize::MAX)] {
                let ts: Vec<_> = tier
                    .times
                    .iter()
                    .filter(|t| (lo..hi).contains(&t.0))
                    .collect();
                if let Some(max) = ts.iter().map(|t| t.1).max() {
                    let sum: std::time::Duration = ts.iter().map(|t| t.1).sum();
                    let ir: usize = ts.iter().map(|t| t.0).sum();
                    eprintln!(
                        "note: --tier=clift: {lo}..{hi} IR instructions: {} functions, {} IR instructions, mean {} us, max {} us",
                        ts.len(),
                        ir,
                        (sum / ts.len() as u32).as_micros(),
                        max.as_micros()
                    );
                }
            }
        }
        rt.set_tier(std::sync::Arc::new(tier));
        return Ok(None);
    }
    // The tier-up policy of `--tier=auto` with Cranelift as its only code generator.
    let heat = a
        .tier_heat
        .map_or(yavs_llvm::Heat::DEFAULT, |[base, per_instr, period]| {
            yavs_llvm::Heat {
                base,
                per_instr,
                period,
                budget: 0,
            }
        });
    let prog = rt.prog.clone();
    let verbose = a.verbose;
    let note: yavs_llvm::Note = Box::new(move |f, _level, why| {
        if !verbose {
            return;
        }
        let name = prog.func(f).map_or("?", |f| f.name.as_str());
        let n = prog.func(f).map_or(0, |f| f.code.len());
        match why {
            Ok(t) => eprintln!(
                "note: `{name}` ({n} IR instructions) is hot: compiled by --tier=clift-auto in {} us",
                t.as_micros()
            ),
            Err(why) => eprintln!("note: `{name}` is hot but stays interpreted: {why}"),
        }
    });
    let first = yavs_clift::First::new().map_err(bug)?;
    if let Some(ps) = yavs_clift::PostSet::new(rt.prog.clone(), true) {
        ps.install();
    }
    let t = yavs_llvm::AutoTier::start_deferred_with(
        rt,
        yavs_llvm::OptLevel::O2,
        a.tier_max_size,
        heat,
        note,
        yavs_llvm::Gens {
            main: None,
            first: Some(Box::new(first)),
        },
    )
    .map_err(|e| bug(e.to_string()))?;
    Ok(Some(StopTierUp::Llvm(t)))
}

/// `--tier=auto3`: `yavs_llvm::AutoTier` with Cranelift as its first level (functions are
/// translated by Cranelift once warm, compiled again by LLVM at O2 once they stay hot).
#[cfg(all(feature = "llvm", feature = "clift"))]
fn use_auto3(
    rt: &yavs_interp::Runtime,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    clift_cache(a);
    // (The O2 compiles of LLVM link from its own store, as under `--tier=auto`.)
    let work = a.common.work.split_once(':').map_or("", |(_, d)| d);
    if !a.common.no_cache && !work.is_empty() {
        yavs_llvm::set_cache(Some(std::path::Path::new(work)));
    }
    let bug = |e: String| {
        Box::new(
            Diagnostic::error("E0121", "the compiled tiers cannot compile the design")
                .with_note(e)
                .with_note("this is a bug in yavs"),
        )
    };
    let heat = a
        .tier_heat
        .map_or(yavs_llvm::Heat::DEFAULT, |[base, per_instr, period]| {
            yavs_llvm::Heat {
                base,
                per_instr,
                period,
                budget: 0,
            }
        });
    let prog = rt.prog.clone();
    let verbose = a.verbose;
    let note: yavs_llvm::Note = Box::new(move |f, level, why| {
        if !verbose {
            return;
        }
        let name = prog.func(f).map_or("?", |f| f.name.as_str());
        let n = prog.func(f).map_or(0, |f| f.code.len());
        let by = if level == yavs_llvm::OptLevel::O2 {
            "LLVM O2"
        } else {
            "Cranelift"
        };
        match why {
            Ok(t) => eprintln!(
                "note: `{name}` ({n} IR instructions) is hot: compiled by --tier=auto3 with {by} in {} us",
                t.as_micros()
            ),
            Err(why) => eprintln!("note: `{name}` is hot but stays interpreted: {why}"),
        }
    });
    let first = yavs_clift::First::new().map_err(bug)?;
    // `YAVS_MIXED=1`, for tests: LLVM compiles the functions of even id and Cranelift the others
    // (as the only generator), so compiled callers and callees of both kinds call each other.
    if std::env::var_os("YAVS_MIXED").is_some() {
        let t = yavs_llvm::AutoTier::start_deferred_with(
            rt,
            yavs_llvm::OptLevel::O2,
            a.tier_max_size,
            heat,
            note,
            yavs_llvm::Gens {
                main: Some(Box::new(Mixed {
                    llvm: yavs_llvm::LlvmGenerator::new(),
                    clift: first,
                })),
                first: None,
            },
        )
        .map_err(|e| bug(e.to_string()))?;
        return Ok(Some(StopTierUp::Llvm(t)));
    }
    let t = yavs_llvm::AutoTier::start_deferred(
        rt,
        yavs_llvm::OptLevel::O0,
        a.tier_max_size,
        heat,
        note,
        Some(Box::new(first)),
    )
    .map_err(|e| bug(e.to_string()))?;
    Ok(Some(StopTierUp::Llvm(t)))
}

/// The generator of `YAVS_MIXED=1` (`use_auto3`): LLVM for the functions of even id, Cranelift for
/// the others.
#[cfg(all(feature = "llvm", feature = "clift"))]
struct Mixed {
    llvm: yavs_llvm::LlvmGenerator,
    clift: yavs_clift::First,
}

#[cfg(all(feature = "llvm", feature = "clift"))]
impl yavs_llvm::Generator for Mixed {
    fn open(&mut self) -> Result<(), String> {
        self.llvm.open()
    }

    fn tree(&mut self, ids: &[yavs_ir::FuncId]) {
        self.clift.tree(ids);
    }

    fn compile(
        &mut self,
        prog: &yavs_ir::Program,
        id: yavs_ir::FuncId,
        f: &yavs_ir::Func,
        level: yavs_llvm::OptLevel,
        part: yavs_llvm::native::Part,
    ) -> Result<yavs_llvm::native::Compiled, String> {
        if id.0.is_multiple_of(2) {
            self.llvm.compile(prog, id, f, level, part)
        } else {
            self.clift.compile(prog, id, f, level, part)
        }
    }
}

/// A name for this run in the profile of `--tier=auto`: a digest of the command line.
#[cfg(any(feature = "llvm", feature = "clift"))]
fn run_id() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    std::env::args_os().skip(1).for_each(|a| a.hash(&mut h));
    format!("{:016x}", h.finish())
}

/// Set up the compiled tier (`--tier`): compile the program now or start tier-up, stopped when
/// the returned guard drops.
fn use_tier(
    rt: Option<&yavs_interp::Runtime>,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    if a.tier() == TierArg::Auto3 {
        #[cfg(all(feature = "llvm", feature = "clift"))]
        return rt.map_or(Ok(None), |rt| use_auto3(rt, a));
        #[cfg(not(all(feature = "llvm", feature = "clift")))]
        return Err(Box::new(
            Diagnostic::error(
                "E0120",
                "`--tier=auto3` needs a yavs built with the LLVM and Cranelift tiers",
            )
            .with_note("this yavs was built without the `llvm` and `clift` cargo features")
            .with_help("use `--tier=interp`, or build yavs with `--features llvm,clift`"),
        ));
    }
    if matches!(a.tier(), TierArg::Clift | TierArg::CliftAuto) {
        #[cfg(feature = "clift")]
        return rt.map_or(Ok(None), |rt| use_clift(rt, a));
        #[cfg(not(feature = "clift"))]
        return Err(Box::new(
            Diagnostic::error(
                "E0120",
                "`--tier=clift` needs a yavs built with the Cranelift tier",
            )
            .with_note("this yavs was built without the `clift` cargo feature")
            .with_help(
                "use `--tier=interp`, or install a yavs release archive or build yavs with \
                 `--features clift`",
            ),
        ));
    }
    use_llvm_tier(rt, a)
}

/// Set up `--tier=llvm` (compile the program now) or `--tier=auto` (start tier-up).
#[cfg(feature = "llvm")]
fn use_llvm_tier(
    rt: Option<&yavs_interp::Runtime>,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    let Some(rt) = rt else {
        return Ok(None);
    };
    let bug = |e: yavs_llvm::Error| {
        Box::new(
            Diagnostic::error("E0121", "the LLVM tier cannot compile the design")
                .with_note(e.to_string())
                .with_note("this is a bug in yavs"),
        )
    };
    let opt = yavs_llvm::OptLevel::O2;
    // Compiled code is kept in the compilation cache of the work library (#505).
    let work = a.common.work.split_once(':').map_or("", |(_, d)| d);
    if a.tier() != TierArg::Interp && !a.common.no_cache && !work.is_empty() {
        yavs_llvm::set_cache(Some(std::path::Path::new(work)));
        // The run-to-run profile of `--tier=auto` is of this command line (#754).
        yavs_llvm::set_run_id(Some(&run_id()));
    }
    match a.tier() {
        TierArg::Interp | TierArg::Clift | TierArg::CliftAuto | TierArg::Auto3 => Ok(None),
        TierArg::Llvm => {
            // A missing library is a problem of the installation, not a bug in the tier.
            yavs_llvm::load_library().map_err(|e| {
                Box::new(
                    Diagnostic::error("E0127", "the LLVM library cannot be loaded")
                        .with_note(e.to_string())
                        .with_help(
                            "keep the `lib` directory of the release archive next to `yavs`, install LLVM 21, or use `--tier=interp`",
                        ),
                )
            })?;
            let tier = yavs_llvm::compile(&rt.prog, opt, a.tier_max_size).map_err(bug)?;
            use_llvm(rt, tier, a.verbose);
            Ok(None)
        }
        TierArg::Auto => {
            let heat = a
                .tier_heat
                .map_or(yavs_llvm::Heat::DEFAULT, |[base, per_instr, period]| {
                    yavs_llvm::Heat {
                        base,
                        per_instr,
                        period,
                        budget: 0,
                    }
                });
            let prog = rt.prog.clone();
            let verbose = a.verbose;
            let note: yavs_llvm::Note = Box::new(move |f, level, why| {
                if !verbose {
                    return;
                }
                let name = prog.func(f).map_or("?", |f| f.name.as_str());
                let n = prog.func(f).map_or(0, |f| f.code.len());
                match why {
                    Ok(t) => eprintln!(
                        "note: `{name}` ({n} IR instructions) is hot: compiled by --tier=auto at {level:?} in {} ms",
                        t.as_millis()
                    ),
                    Err(why) => {
                        eprintln!("note: `{name}` is hot but stays interpreted: {why}")
                    }
                }
            });
            let t = yavs_llvm::AutoTier::start_deferred(
                rt,
                // The hidden `--tier-heat` is for tests and tuning: O0 first, O2 for the hot ones.
                if a.tier_heat.is_some() {
                    yavs_llvm::OptLevel::O0
                } else {
                    yavs_llvm::OptLevel::O2
                },
                a.tier_max_size,
                heat,
                note,
                None,
            )
            .map_err(|e| bug(yavs_llvm::Error(e.0)))?;
            Ok(Some(StopTierUp::Llvm(t)))
        }
    }
}

/// Run the compiled program from now on (`--tier=llvm`).
#[cfg(feature = "llvm")]
fn use_llvm(rt: &yavs_interp::Runtime, tier: yavs_llvm::LlvmTier, verbose: bool) {
    crate::counters::llvm_compiled(
        tier.inline,
        tier.interpreted.iter().map(|k| k.1).sum(),
        tier.native_frames(),
    );
    if verbose {
        for (f, why) in &tier.notes {
            let name = rt.prog.func(*f).map_or("?", |f| f.name.as_str());
            eprintln!("note: `{name}` stays interpreted under --tier=llvm: {why}");
        }
        let n: usize = tier.interpreted.iter().map(|k| k.1).sum();
        let kinds: Vec<String> = tier
            .interpreted
            .iter()
            .map(|(k, c)| format!("{k} {c}"))
            .collect();
        eprintln!(
            "note: --tier=llvm: {} instructions compiled, {n} executed by the interpreter ({}); {} functions callable on native frames",
            tier.inline,
            kinds.join(", "),
            tier.native_frames()
        );
        let (hits, stored) = yavs_llvm::cache_stats();
        eprintln!(
            "note: --tier=llvm: {hits} modules linked from the compilation cache, {stored} compiled and stored"
        );
    }
    rt.set_tier(std::sync::Arc::new(tier));
}

#[cfg(not(feature = "llvm"))]
fn use_llvm_tier(
    _: Option<&yavs_interp::Runtime>,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    let name = match a.tier() {
        TierArg::Llvm => "llvm",
        TierArg::Auto => "auto",
        TierArg::Interp | TierArg::Clift | TierArg::CliftAuto | TierArg::Auto3 => return Ok(None),
    };
    Err(Box::new(
        Diagnostic::error(
            "E0120",
            format!("`--tier={name}` needs a yavs built with the LLVM tier"),
        )
        .with_note("this yavs was built without the `llvm` cargo feature")
        .with_help(if cfg!(feature = "clift") {
            "use `--tier=auto` or `--tier=clift`, which this yavs has, or `--tier=interp`; \
             `--tier=llvm` needs a yavs built with `--features llvm` (needs LLVM 21)"
        } else {
            "use `--tier=interp`, or build yavs with `--features clift` (Cranelift, as the \
             release archives) or `--features llvm` (needs LLVM 21)"
        }),
    ))
}

/// Everything from the libraries to the design a run builds: the first time, and at each reload
/// of a console session.
fn prepare(
    a: &RunArgs,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
    lazy_pool: bool,
) -> Result<Prepared, (Out, u8)> {
    let kind = |k| a.cov.is_some() && a.cov_kind.contains(&k);
    let lowering_counters = kind(CovKindArg::Statement)
        || kind(CovKindArg::Branch)
        || kind(CovKindArg::Condition)
        || kind(CovKindArg::Assert)
        || kind(CovKindArg::Expression)
        || kind(CovKindArg::Combination);
    let lowering = yavs_elab::LowerOptions {
        coverage: kind(CovKindArg::Statement) || kind(CovKindArg::Branch),
        cov_condition: kind(CovKindArg::Condition),
        cov_assert: kind(CovKindArg::Assert),
        cov_expression: kind(CovKindArg::Expression),
        cov_combination: kind(CovKindArg::Combination),
        cov_directives: a.common.coverage_directives == DirectivesArg::Apply,
        ..yavs_elab::LowerOptions::default()
    };
    let libs = match install_foreign(&a.common.load) {
        Ok(libs) => libs,
        Err(d) => {
            let mut out = Out::new(format, color);
            out.emit(&SourceMap::default(), &d);
            return Err((out, 2));
        }
    };
    // A debugger or VHPI plugin may force or deposit any port: those runs keep ports apart.
    let collapse = !a.debugged() && a.vhpi.is_empty();
    let (mut out, db, e) = elaborate(
        &a.common,
        lowering,
        matches!(a.tier(), TierArg::Llvm | TierArg::Clift),
        collapse,
        format,
        color,
        encoding,
    )?;
    if a.common.print_hierarchy {
        print_hierarchy(&db, &e);
    }
    let none = SourceMap::default();
    let flag = if a.dap.is_some() {
        "--dap"
    } else {
        "--interactive"
    };
    let unsupported = |what: &str| {
        Diagnostic::error("E0199", format!("{what} is not implemented yet in yavs"))
            .with_note("this is a limitation of yavs, not an error in the design")
            .with_help(format!("run without `--cov` or without `{flag}`"))
    };
    if a.debugged() {
        // A debugger session runs the interpreter whatever `--tier` says, but a tier this build
        // lacks is E0120 as without `--dap` (#835): nothing is compiled, so no runtime is given.
        if let Err(d) = use_tier(None, a) {
            out.emit(&none, &d);
            return Err((out, 2));
        }
        if a.profile.is_some() {
            out.emit(
                &none,
                &Diagnostic::error(
                    "E0125",
                    format!("`--profile` cannot be combined with `{flag}`"),
                )
                .with_note("a stop at a breakpoint would be sampled as time in the stopped process")
                .with_help(format!("profile a run without `{flag}`")),
            );
            return Err((out, 2));
        }
        if a.cov.is_some() {
            out.emit(
                &none,
                &unsupported(&format!("`--cov` together with `{flag}`")),
            );
            return Err((out, 2));
        }
        if a.threads > 1 {
            out.emit(
                &none,
                &Diagnostic::warning("W0109", format!("`--threads` is ignored with `{flag}`"))
                    .with_note("a stop inside a process needs the simulation on one thread"),
            );
        }
    }
    let resolution = match a.resolution {
        ResolutionArg::Fs => yavs_kernel::Unit::Fs,
        ResolutionArg::Ps => yavs_kernel::Unit::Ps,
        ResolutionArg::Ns => yavs_kernel::Unit::Ns,
        ResolutionArg::Us => yavs_kernel::Unit::Us,
        ResolutionArg::Ms => yavs_kernel::Unit::Ms,
        ResolutionArg::Sec => yavs_kernel::Unit::Sec,
    };
    // Plugins load after elaboration and before initialisation; a load failure is fatal.
    if !a.vhpi.is_empty() {
        yavs_vhpi::set_resolution(resolution);
        yavs_vhpi::set_design(&db, &e);
        let mut argv = vec!["yavs".to_string()];
        argv.extend(a.plusargs.iter().cloned());
        yavs_vhpi::set_argv(argv);
        yavs_vhpi::set_reset_refusal(a.no_vhpi_reset());
        yavs_vhpi::set_shuffle_seed(a.shuffle);
        for spec in &a.vhpi {
            let p: yavs_vhpi::PluginSpec = spec.parse().unwrap_or_else(|e| match e {});
            let loaded = yavs_vhpi::load(&p).map_err(|err| err.to_diagnostic());
            if let Err(d) = loaded {
                out.emit(&none, &d);
                return Err((out, 2));
            }
        }
    }
    if !a.vhpi.is_empty() {
        yavs_vhpi::start_of_elaboration();
    }
    // The models of VHPI foreign architectures run now, before the build (#330).
    let mut e = e;
    if !e.foreign_archs.is_empty() {
        match yavs_vhpi::elaborate_foreign(&e) {
            Ok(procs) => e.ext_procs = procs,
            Err(d) => {
                out.emit(&none, &d);
                return Err((out, 2));
            }
        }
    }
    let others_free = !a.debugged() && a.vhpi.is_empty();
    let observed_free = a.cov.is_none() && others_free;
    let cov_counters = lowering_counters;
    // Wide signals nothing asks the history of keep only each block's last event, unless
    // something observes the run; hidden test aid: `YAVS_LEAN_EVENTS=off` keeps all histories.
    if !observed_free || std::env::var("YAVS_LEAN_EVENTS").is_ok_and(|v| v == "off") {
        e.lean.clear();
    }
    // Hidden test aid: `YAVS_CODES=all` keeps the byte codes of every logic signal.
    if std::env::var("YAVS_CODES").is_ok_and(|v| v == "all") {
        e.no_codes.clear();
    }
    let idle_mode = std::env::var("YAVS_IDLE_SKIP").unwrap_or_default();
    let cfg = yavs_kernel::Config {
        threads: a.threads,
        shuffle: a.shuffle,
        max_deltas: a.max_deltas,
        resolution,
        stop_severity: match a.exit_severity.unwrap_or(SeverityArg::Failure) {
            SeverityArg::Note => yavs_kernel::Severity::Note,
            SeverityArg::Warning => yavs_kernel::Severity::Warning,
            SeverityArg::Error => yavs_kernel::Severity::Error,
            SeverityArg::Failure => yavs_kernel::Severity::Failure,
        },
        // A process left suspended by its edge trigger would be visible to a debugger stepping
        // through it and to foreign callbacks. Coverage accepts it: the statement of a pure
        // edge test counts the activations at its edge only (#707).
        edge_triggers: !a.debugged() && a.vhpi.is_empty(),
        // Coverage keeps the count-neutral skips (user, 2026-10-24, #707): an unchanged assignment
        // queues no transaction under every kind, but the idle skip leaves activations out and
        // so is off when statement, branch, condition or expression counters count them.
        skip_unchanged: others_free && !db.facts().reads_activity,
        // Hidden switches for comparing runs: `YAVS_IDLE_SKIP=off` runs every activation,
        // `check` runs the ones the kernel would skip and fails when one does something else.
        idle_skip: idle_mode != "off" && !cov_counters,
        idle_check: idle_mode == "check",
        // `YAVS_CELLS=off` lists the exact runs of changed elements for every waited part.
        cells: !std::env::var("YAVS_CELLS").is_ok_and(|v| v == "off"),
        // `YAVS_POOL_ALL=1` hands every batch of two or more processes to the thread pool
        // (`--threads` above 1): the determinism runs exercise the pooled path with it.
        pool_all: std::env::var("YAVS_POOL_ALL").is_ok_and(|v| v == "1"),
        // Under a token server the worker threads start with the first token (#879).
        lazy_pool,
    };
    if let Some(seed) = a.shuffle {
        out.emit(&none, &shuffle_note(seed));
    }
    let stop = match &a.stop_time {
        None => yavs_kernel::Time::MAX,
        Some(t) => match parse_time(t) {
            Ok(t) => t,
            Err(m) => {
                // A bare number is the common mistake: say so instead of the parser's words.
                let bare = t.trim().chars().all(|c| c.is_ascii_digit() || c == '.');
                let d = Diagnostic::error("E0105", format!("invalid `--stop-time={t}`"));
                out.emit(
                    &none,
                    &if bare && !t.trim().is_empty() {
                        d.with_note("a time needs a unit")
                            .with_help("write the unit after the number, for example `--stop-time=10ns` (fs, ps, ns, us, ms, sec, min, hr)")
                    } else {
                        let d = d.with_note(m);
                        match time_unit_help(t) {
                            Some(h) => d.with_help(h),
                            None => d,
                        }
                    },
                );
                return Err((out, 2));
            }
        },
    };
    let mut e = e;
    let bound = bind_foreign(&libs, &mut e, &mut out);
    if let Err(d) = bound {
        out.emit(&none, &d);
    }
    if out.errors > 0 {
        return Err((out, 2));
    }
    if let Some(d) = yavs_elab::check_time_resolution(&db, cfg.resolution) {
        out.emit(&db.sources, &d);
        return Err((out, 2));
    }
    if !a.vhpi.is_empty() {
        yavs_vhpi::end_of_elaboration();
    }
    Ok(Prepared {
        out,
        db,
        e,
        cfg,
        stop,
    })
}

/// A console session's `reload`: the run's command line, these flags added, elaborated again.
fn reloader(
    argv: Vec<String>,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> yavs_dap::ReloadFn {
    Box::new(move |extra| {
        let mut args = argv.clone();
        args.extend_from_slice(extra);
        let mut a = crate::run_args(&args)?;
        a.resolve_cov().map_err(|d| d.message)?;
        match prepare(&a, format, color, encoding, false) {
            Ok(p) => Ok(yavs_dap::Loaded {
                db: Arc::new(p.db),
                e: Arc::new(p.e),
                cfg: p.cfg,
                stop: p.stop,
            }),
            Err((out, code)) => {
                out.finish(code);
                Err("the design did not elaborate: the diagnostics are on standard error".into())
            }
        }
    })
}

/// The design of a run, ready to build.
struct Prepared {
    out: Out,
    db: yavs_sem::Db,
    e: yavs_elab::Elaborated,
    cfg: yavs_kernel::Config,
    stop: yavs_kernel::Time,
}

/// Warning W0118: the tokens are gone; the result of the run does not depend on them.
fn token_gone(why: String) -> Diagnostic {
    Diagnostic::warning(
        "W0118",
        format!(
            "the token server in `{}` cannot be used; the run goes on with one thread",
            crate::tokens::VAR
        ),
    )
    .with_note(why)
}

pub fn run(
    a: RunArgs,
    argv: Vec<String>,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    let plugins = !a.vhpi.is_empty();
    let status = run_design(a, argv, format, color, encoding);
    if plugins {
        yavs_vhpi::end_of_tool();
    }
    status
}

fn run_design(
    mut a: RunArgs,
    argv: Vec<String>,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    if let Err(d) = a.resolve_cov() {
        let mut out = Out::new(format, color);
        out.emit(&SourceMap::default(), &d);
        return out.finish(2);
    }
    // `--surfer` without `--wave`: `<top>.fst` beside the work library directory.
    if a.surfer.is_some() && a.wave.is_none() {
        let dir = a.common.work.split_once(':').map_or("", |(_, d)| d);
        let unit = a.common.top.split(['.', '(']).next().unwrap_or_default();
        let dir = std::path::Path::new(dir).parent().unwrap_or("".as_ref());
        a.wave = Some(dir.join(format!("{unit}.fst")).display().to_string());
        a.wave_default = true;
    }
    if a.exit_on_stdin_close
        && a.dap != Some(yavs_dap::Transport::Stdio)
        && let Err(err) = exit_when_stdin_ends(!a.common.load.is_empty() || !a.vhpi.is_empty())
    {
        eprintln!("error: cannot watch standard input for `--exit-on-stdin-close`: {err}");
        return ExitCode::from(2);
    }
    // First: a token server that is not `<port>:<secret>` or refuses the secret stops the run before
    // any work; one that cannot be reached is a warning, below, and the run has one thread.
    let (tokens, token_warning) =
        match (a.threads > 1 && !a.debugged()).then(crate::tokens::connect) {
            Some(Err(why)) => {
                let mut out = Out::new(format, color);
                out.emit(
                    &SourceMap::default(),
                    &Diagnostic::error(
                        "E0128",
                        format!(
                            "the token server in `{}` cannot be used",
                            crate::tokens::VAR
                        ),
                    )
                    .with_note(why)
                    .with_help(format!(
                        "a test runner sets `{}` for each simulation while its token server runs; \
                     unset it to run without one",
                        crate::tokens::VAR
                    )),
                );
                return out.finish(2);
            }
            Some(Ok(crate::tokens::Server::Pool(p))) => (Some(p), None),
            Some(Ok(crate::tokens::Server::Unreachable(why))) => (None, Some(why)),
            _ => (None, None),
        };
    if a.time_report {
        crate::timing::enable();
    }
    // Before elaboration: its constant evaluation reports warnings too.
    if a.ieee_warnings == Some(OnOffArg::Off) {
        yavs_interp::suppress_ieee_warnings(yavs_sem::default_stdlib().join("ieee"));
    }
    // Before elaboration: its constant evaluation collects with the same threshold.
    if let Some(b) = a.gc_threshold {
        yavs_interp::set_default_gc_threshold(b);
    }
    // A debugger reads and writes the variables of subprograms in their frame slots: keep them
    // there, before any function is optimised.
    if a.debugged() {
        yavs_interp::keep_variables();
    }
    // Eager Cranelift compiles the whole program before time 0 like `--tier=llvm` (no specialised
    // copies, layout slots); the tiers that tier up with LLVM get the layout slots, which also keep
    // the 32-byte blocks the interpreter fuses in the frame, the form compiled code is faster with
    // (#756). `clift-auto` keeps only the blocks in the frame: its code does not use the slots,
    // which cost process-frame memory (user, 2026-10-26); `YAVS_CLIFT_LAYOUT_SLOTS=1` adds them.
    match a.tier() {
        TierArg::Llvm | TierArg::Clift => yavs_interp::compiled_tier(),
        TierArg::Auto | TierArg::Auto3 => yavs_interp::layout_slots(),
        TierArg::CliftAuto if std::env::var("YAVS_CLIFT_LAYOUT_SLOTS").is_ok_and(|v| v == "1") => {
            yavs_interp::layout_slots()
        }
        TierArg::CliftAuto => yavs_interp::frame_blocks(),
        TierArg::Interp => {}
    }
    // A plugin may deposit into process variables between activations, and the waveform
    // recorder reads them from their slots.
    if !a.vhpi.is_empty() || a.wave_vars() {
        yavs_interp::external_variables();
    }
    // Before any plugin or foreign library loads: their prints must not reach the protocol.
    if a.dap == Some(yavs_dap::Transport::Stdio) {
        match yavs_vhpi::divert_stdout() {
            Ok(f) => yavs_dap::set_stdout(f),
            Err(err) => {
                eprintln!("error: cannot protect standard output for `--dap=stdio`: {err}");
                return ExitCode::from(2);
            }
        }
    }
    let kind = |k| a.cov.is_some() && a.cov_kind.contains(&k);
    let Prepared {
        mut out,
        mut db,
        mut e,
        cfg,
        stop,
    } = match prepare(&a, format, color, encoding, tokens.is_some()) {
        Ok(p) => p,
        Err((out, code)) => {
            let status = out.finish(code);
            // Nobody but the client that started this process sees it (#934).
            if a.dap == Some(yavs_dap::Transport::Stdio) {
                yavs_dap::refuse(&out.error_text, code);
            }
            return status;
        }
    };
    let none = SourceMap::default();
    if let Some(why) = token_warning {
        out.emit(&none, &token_gone(why));
    }
    let cfg_profile = a
        .profile
        .is_some()
        .then(yavs_kernel::profile::Profile::install);
    // A debug session always counts, for the console's `activity`.
    let activity =
        (a.activity.is_some() || a.debugged()).then(yavs_kernel::profile::Activity::install);
    if a.debugged() {
        let (db, e) = (Arc::new(db), Arc::new(e));
        // One signal thread, for the simulation of the moment.
        let live: Arc<std::sync::Mutex<Option<yavs_kernel::Interrupt>>> = Arc::default();
        let mut hooked = false;
        let view: ViewSlot = Arc::default();
        let viewer = view.clone();
        let attach = |sim: &mut yavs_kernel::SimControl,
                      built: Option<&yavs_elab::Built>,
                      db: &Arc<yavs_sem::Db>,
                      e: &Arc<yavs_elab::Elaborated>| {
            let rec = observers(&a, db, e, sim, built, &mut out, &view)?;
            if let Some(ac) = &activity {
                sim.subscribe(ac.observer());
            }
            *live.lock().unwrap_or_else(|p| p.into_inner()) = Some(sim.interrupt());
            if rec.is_some() && a.dap.is_some() && !std::mem::replace(&mut hooked, true) {
                // Ctrl-C ends the run between cycles so the file is finished (rule 11).
                let mut hooks = yavs_wave::interrupt::Handlers::default();
                let live = live.clone();
                let stop = move || {
                    if let Some(i) = &*live.lock().unwrap_or_else(|p| p.into_inner()) {
                        i.stop();
                    }
                };
                ctrl_c_stops(&mut hooks, stop, Arc::default(), true);
                yavs_wave::interrupt::start(hooks).map_err(handler_error)?;
            }
            let viewer = view.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let viewer = viewer.map(|v| v as Arc<dyn yavs_dap::Viewer>);
            let wave_file = rec.as_ref().map(|r| r.0.clone());
            let (finish, sync) = match rec {
                Some((path, rec)) => {
                    let r = rec.clone();
                    let finish: yavs_dap::Finish = Box::new(move |sim| {
                        rec.finish(sim).err().map(|err| wave_error(&path, err))
                    });
                    let sync: yavs_dap::Sync = Box::new(move |stop| match stop {
                        Some((now, signals)) => r.sync_at(now, signals),
                        None => r.sync(),
                    });
                    (Some(finish), Some(sync))
                }
                None => (None, None),
            };
            Ok(yavs_dap::Attached {
                finish,
                sync,
                viewer,
                wave_file,
                activity: activity.clone(),
            })
        };
        let report: yavs_dap::Report = Box::new(|stats, rt| {
            crate::counters::report(stats, rt);
            yavs_interp::census_dump(40);
        });
        let options = yavs_dap::Options {
            no_restart: (!a.vhpi.is_empty() || !a.common.load.is_empty()).then(|| {
                "restart cannot reset the state of a library loaded with `--vhpi` or `--load`: \
                 start the run again"
                    .to_string()
            }),
            interactive: (!a.vhpi.is_empty()).then_some(yavs_vhpi::interactive as fn(bool)),
            reload: Some(reloader(argv, format, color, encoding)),
            hook: std::env::var(yavs_dap::HOOK_VAR).ok(),
            source_map: a.source_map.clone(),
            layout: layout_path(&a),
            hang_report: a.hang_report,
            assert_summary: a.assert_summary,
            activity: a.activity.clone(),
        };
        let loaded = yavs_dap::Loaded {
            db: db.clone(),
            e: e.clone(),
            cfg,
            stop,
        };
        let r = match a.dap {
            Some(t) => yavs_dap::serve(loaded, t, options, attach, report),
            None => crate::console::serve(
                loaded,
                &a.do_commands,
                a.interactive,
                color,
                options,
                attach,
                report,
            ),
        };
        // The file is finished: Surfer shows the whole run; the status never depends on it.
        view_end(&viewer, format, color);
        return match r {
            Ok(code) => out.finish(code),
            Err(d) => {
                out.emit(&db.sources, &d);
                out.finish(2)
            }
        };
    }
    // Nothing below reads the design but its source map unless a plugin, the waveform
    // recorder or coverage does: free the analysis results before the runtime is built (#409).
    if a.vhpi.is_empty() && a.wave.is_none() && a.cov.is_none() {
        db.release_analysis();
    }
    // `vhpi_control(vhpiReset)` builds it again (#832).
    let cfg_again = (!a.vhpi.is_empty()).then(|| cfg.clone());
    let (mut sim, mut built) = match e.build_full_once(cfg) {
        Ok(s) => s,
        // Package and instance initialisation runs VHDL code: report it with its VHDL stack.
        Err(err) => {
            out.emit(&db.sources, &e.runtime_diagnostic(&err));
            return out.finish(if err.exit_status() == 101 { 3 } else { 2 });
        }
    };
    // Where the processes are, for the diagnostics of a stuck run; read before the model is freed.
    let procs = e.proc_table();
    for w in e.no_wait_warnings(&procs) {
        out.emit(&db.sources, &w);
    }
    // Held to the end of the run: `end` or a drop ends tier-up.
    let mut tier_up = match use_tier(built.as_ref().map(|b| &*b.rt), &a) {
        Ok(g) => g,
        Err(d) => {
            let code = if matches!(d.code, "E0120" | "E0127") {
                2
            } else {
                3
            };
            out.emit(&none, &d);
            return out.finish(code);
        }
    };
    if cfg_profile.is_some()
        && a.tier() == TierArg::Interp
        && let Some(b) = &built
    {
        crate::profile::watch_interpreter(&b.rt);
    }
    // Only the design writes to standard output: no plugin, no foreign code.
    let foreign =
        e.ir.as_ref()
            .is_some_and(|ir| ir.program.foreign.iter().next().is_some());
    if a.vhpi.is_empty() && a.common.load.is_empty() && !foreign {
        crate::stdout::buffer();
    }
    if let Some(p) = &tokens {
        sim.set_tokens(p.clone());
    }
    sim.subscribe(Box::new(Printer));
    if let Some(ac) = &activity {
        sim.subscribe(ac.observer());
    }
    if a.assert_summary
        && let Some(b) = &built
    {
        b.rt.keep_assert_sites();
    }
    let activity_names = activity.as_ref().map(|_| yavs_elab::ActivityNames {
        procs: (0..sim.process_count())
            .map(|p| {
                sim.process_name(yavs_kernel::ProcId(p as u32))
                    .unwrap_or("?")
                    .to_string()
            })
            .collect(),
        signals: (0..sim.signal_count())
            .map(|s| {
                sim.signal_name(yavs_kernel::SignalId(s as u32))
                    .unwrap_or("?")
                    .to_string()
            })
            .collect(),
    });
    let on_pipe = sim.interrupt();
    crate::stdout::on_broken(move || on_pipe.stop());
    let mut rt = built.as_ref().map(|b| b.rt.clone());
    // Toggle coverage watches the value-change bus; without it nothing is subscribed.
    let toggles = kind(CovKindArg::Toggle).then(|| {
        let (t, skipped) = yavs_cov::toggle::toggles(&db, &e, a.cov_toggle_max);
        for s in skipped {
            out.emit(
                &none,
                &Diagnostic::warning(
                    "W0115",
                    format!("`{}` is not in the toggle coverage", s.path),
                )
                .with_note(format!(
                    "it has {} scalars, more than `--cov-toggle-max={}`",
                    s.elems, a.cov_toggle_max
                )),
            );
        }
        sim.subscribe(Box::new(t.clone()));
        t
    });
    // State machine coverage the same way, over the machines detection finds.
    let mut unmeasured = std::collections::BTreeSet::new();
    let fsm = kind(CovKindArg::Fsm).then(|| {
        let reading = yavs_cov::fsm::Reading {
            translate_off: a.cov_exclude_translate_off,
            directives: a.common.coverage_directives == DirectivesArg::Apply,
        };
        let (f, missed, warns) = yavs_cov::fsm::fsm(&db, &e, reading);
        for d in &warns {
            out.emit(&db.sources, d);
        }
        for (span, text) in missed {
            let src = db.sources.get(span.file);
            unmeasured.insert((
                src.path.clone(),
                src.line_col(span.start).0 as u32,
                text.clone(),
            ));
            out.emit(
                &db.sources,
                &Diagnostic::warning("W0120", format!("not measured as a state machine: {text}"))
                    .with_primary(span, "this looks like a state machine")
                    .with_help("`yavs --explain W0120` says which forms are measured"),
            );
        }
        sim.subscribe(Box::new(f.clone()));
        f
    });
    let interrupted = Arc::new(AtomicBool::new(false));
    let view: ViewSlot = Arc::default();
    let recorder = match observers(&a, &db, &e, &mut sim, built.as_ref(), &mut out, &view) {
        Ok(r) => r,
        Err(d) => {
            out.emit(&none, &d);
            return out.finish(2);
        }
    };
    // One thread for Ctrl-C/SIGTERM, SIGUSR1 (hang report) and the output flusher (#562).
    // With a recorder the first signal ends the run between cycles so that the file is finished
    // (rule 11). With `--profile` it does so that the profile of the time so far is printed; the
    // exit status stays 130. Else, with output collected, it writes the reports that wait in
    // the buffer out and exits at once.
    let signalled = Arc::new(AtomicBool::new(false));
    // The simulation of the moment: a VHPI reset replaces it.
    let live = Arc::new(std::sync::Mutex::new(sim.interrupt()));
    let current = live.clone();
    let mut hooks = yavs_wave::interrupt::Handlers {
        usr1: Some(crate::stuck::on_request(move || {
            current.lock().unwrap_or_else(|p| p.into_inner()).pause()
        })),
        ..Default::default()
    };
    if crate::stdout::is_on() {
        hooks.tick = Some((crate::stdout::TICK, Box::new(crate::stdout::flush)));
    }
    let launch = tier_up.as_ref().map(|t| t.launcher(format, color));
    // After a VHPI reset the signal thread must not start the old simulation's tier-up.
    let tier_up_current = Arc::new(AtomicBool::new(true));
    let still = tier_up_current.clone();
    hooks.after = launch.clone().map(|(after, f)| {
        let start = move || {
            if still.load(Ordering::SeqCst) {
                f();
            }
        };
        (after, Box::new(start) as _)
    });
    let keep_exit = !a.common.load.is_empty() || !a.vhpi.is_empty();
    if recorder.is_some()
        || cfg_profile.is_some()
        || activity.is_some()
        || a.assert_summary
        || !a.vhpi.is_empty()
    {
        let flag = if recorder.is_some() {
            &interrupted
        } else {
            &signalled
        };
        let current = live.clone();
        let stop = move || {
            yavs_vhpi::note_interrupt();
            current.lock().unwrap_or_else(|p| p.into_inner()).stop();
        };
        ctrl_c_stops(&mut hooks, stop, flag.clone(), keep_exit);
    } else if crate::stdout::is_on() {
        hooks.first = Some(Box::new(move || {
            crate::stdout::flush();
            exit_after_signal(130, keep_exit);
        }));
    }
    if let Err(err) = yavs_wave::interrupt::start(hooks) {
        if recorder.is_some() {
            out.emit(&none, &handler_error(err));
            return out.finish(2);
        }
        // No helper thread to start it later.
        if let Some((_, f)) = &launch {
            f();
        }
        crate::stdout::unbuffer();
        out.emit(
            &none,
            &Diagnostic::warning(
                "W0116",
                "cannot install the signal handlers (Ctrl-C, SIGUSR1 hang reports)",
            )
            .with_note(err.to_string()),
        );
    }
    let regions: Vec<String> = cfg_profile
        .iter()
        .flat_map(|_| &e.procs)
        .map(|p| {
            let region = e.hier.scope(p.scope).parent.unwrap_or(p.scope);
            e.hier.dotted_in(region, yavs_elab::Case::Lrm)
        })
        .collect();
    // Nothing below reads the elaborated model but its IR (run-time diagnostics) unless a
    // plugin, the waveform recorder or coverage does (#421).
    if a.vhpi.is_empty() && a.wave.is_none() && a.cov.is_none() {
        e.release_for_run();
    }
    crate::timing::mark("init");
    crate::counters::start();
    let sampler = cfg_profile.map(|p| {
        crate::profile::Sampler::start(
            p,
            rt.as_ref().map(|rt| rt.profile_sampler()),
            rt.as_ref().map(|rt| rt.line_sampler()),
            regions,
        )
    });
    let max = a.hang_report.unwrap_or(30);
    let r = loop {
        let r = sim.run_until(stop);
        if r.is_ok()
            && let Some(cfg) = &cfg_again
            && yavs_vhpi::reset_pending()
            && !signalled.load(Ordering::SeqCst)
        {
            // The old simulation goes first: its thread pool and open files.
            drop(sim);
            let (again, b) = match e.build_full(cfg.clone()) {
                Ok(s) => s,
                Err(err) => {
                    out.emit(&db.sources, &e.runtime_diagnostic(&err));
                    return out.finish(if err.exit_status() == 101 { 3 } else { 2 });
                }
            };
            sim = again;
            built = b;
            rt = built.as_ref().map(|b| b.rt.clone());
            tier_up_current.store(false, Ordering::SeqCst);
            tier_up = match use_tier(rt.as_deref(), &a) {
                Ok(g) => g,
                Err(d) => {
                    out.emit(&none, &d);
                    return out.finish(3);
                }
            };
            if let Some(t) = &tier_up {
                (t.launcher(format, color).1)();
            }
            if let Some(p) = &tokens {
                sim.set_tokens(p.clone());
            }
            sim.subscribe(Box::new(Printer));
            if let Err(d) = observers(&a, &db, &e, &mut sim, built.as_ref(), &mut out, &view) {
                out.emit(&none, &d);
                return out.finish(2);
            }
            *live.lock().unwrap_or_else(|p| p.into_inner()) = sim.interrupt();
            yavs_vhpi::reset_done();
            continue;
        }
        if matches!(r, Ok(RunStatus::Paused)) && crate::stuck::take_request() {
            let report = sim.stuck_report();
            let d = e.stuck_diagnostic(&report, yavs_elab::StuckWhy::Requested, &procs, max);
            out.emit(&db.sources, &with_seed(d, a.shuffle));
            continue;
        }
        if a.hang_report.is_some() {
            let why = match &r {
                Ok(RunStatus::Paused) => Some(yavs_elab::StuckWhy::StopTime(stop)),
                Ok(RunStatus::Ended(EndReason::Idle)) => Some(yavs_elab::StuckWhy::Idle),
                _ => None,
            };
            if let Some(why) = why {
                let report = sim.stuck_report();
                if why != yavs_elab::StuckWhy::Idle || crate::stuck::idle_is_stuck(&report) {
                    let d = e.stuck_diagnostic(&report, why, &procs, max);
                    out.emit(&db.sources, &with_seed(d, a.shuffle));
                }
            }
        }
        break r;
    };
    crate::timing::mark("simulate");
    crate::stdout::flush();
    let mut profile_error = None;
    if let (Some(s), Some(file)) = (sampler, &a.profile) {
        profile_error = s
            .finish(rt.as_deref(), &db.sources, file.as_deref())
            .err()
            .map(|err| (file.clone().unwrap_or_default(), err));
    }
    crate::timing::report();
    crate::counters::report(sim.stats(), rt.as_deref());
    yavs_interp::census_dump(40);
    let mut activity_error = None;
    if let (Some(ac), Some(names), Some(file)) = (&activity, &activity_names, &a.activity) {
        let (runs, events) = (ac.runs(), ac.events());
        eprint!(
            "{}",
            yavs_elab::activity_text(&e, &procs, &runs, &events, names, &db.sources)
        );
        if let Some(path) = file {
            let j = yavs_elab::activity_json(&e, &procs, &runs, &events, names, &db.sources);
            activity_error = std::fs::write(path, format!("{j:#}\n"))
                .err()
                .map(|err| (path.clone(), err));
        }
    }
    if a.assert_summary {
        match format {
            MessageFormat::Json => eprintln!(
                "{}",
                yavs_elab::assert_summary_json(rt.as_deref(), &db.sources)
            ),
            _ => eprint!(
                "{}",
                yavs_elab::assert_summary_text(rt.as_deref(), &db.sources)
            ),
        }
    }
    if let Some((path, err)) = activity_error {
        out.emit(
            &none,
            &Diagnostic::error(
                "E0124",
                format!("cannot write the activity report `{path}`"),
            )
            .with_note(err.to_string()),
        );
        return out.finish(1);
    }
    if signalled.load(Ordering::SeqCst) {
        if let Some((path, err)) = profile_error {
            out.emit(
                &none,
                &Diagnostic::error("E0124", format!("cannot write the profile `{path}`"))
                    .with_note(err.to_string()),
            );
        }
        return out.finish(130);
    }
    let mut o = std::io::stdout().lock();
    let code = match r {
        Ok(RunStatus::Ended(EndReason::Stop(c) | EndReason::Finish(c))) => {
            let what = match r {
                Ok(RunStatus::Ended(EndReason::Stop(_))) => "STOP",
                _ => "FINISH",
            };
            let _ = writeln!(
                o,
                "** Note: {}fs+{}: {what} called with status {c}",
                sim.now().as_fs(),
                sim.delta()
            );
            if (0..=125).contains(&c) { c as u8 } else { 1 }
        }
        Ok(RunStatus::Ended(EndReason::Severity(_))) => 1,
        // Ctrl-C, or the recorder stopped the run after a write error (reported below).
        Ok(RunStatus::Ended(EndReason::Requested)) => {
            if interrupted.load(Ordering::Relaxed) {
                let _ = writeln!(
                    o,
                    "** Note: {}fs+{}: simulation interrupted",
                    sim.now().as_fs(),
                    sim.delta()
                );
            }
            1
        }
        Ok(_) => 0,
        Err(err) => {
            let _ = o.flush();
            let d = e.runtime_diagnostic_in(&err, &procs);
            out.emit(&db.sources, &with_seed(d, a.shuffle));
            if err.exit_status() == 101 { 3 } else { 1 }
        }
    };
    let _ = o.flush();
    drop(o);
    let mut code = code;
    if let Some((path, err)) = profile_error {
        out.emit(
            &none,
            &Diagnostic::error("E0124", format!("cannot write the profile `{path}`"))
                .with_note(err.to_string()),
        );
        code = code.max(1);
    }
    if let Some((path, rec)) = recorder
        && let Err(err) = rec.finish(&sim)
    {
        out.emit(&none, &wave_error(&path, err));
        code = code.max(1);
    }
    // After the file is finished; the viewer never changes the status.
    view_end(&view, format, color);
    // A report of severity error or failure fails the run even below `--exit-severity`, unless
    // ClearVhdlAssert cleared the counts since: the status follows the VHDL assert counts of
    // std.env (as the reference simulator; test runners only look at zero or non-zero).
    // Reports made while elaborating (#349) count too.
    use yavs_kernel::Severity::{Error, Failure};
    // Given explicitly, the level is the whole threshold: a run that ends normally after
    // `error` reports below it exits 0 (the test runner passes `failure` and relies on it).
    let counted = if a.exit_severity.is_some() {
        &[Failure][..]
    } else {
        &[Error, Failure][..]
    };
    let failed = counted.iter().any(|&sv| {
        yavs_elab::elaboration_reports(sv) > 0
            || rt.as_ref().is_some_and(|rt| rt.assert_count(sv) > 0)
    });
    let mut code = if code == 0 && failed { 1 } else { code };
    // The coverage of the run so far, whatever its outcome.
    if let Some(path) = &a.cov {
        // A design without processes has no counters.
        let counts = rt.as_ref().map(|rt| rt.coverage()).unwrap_or_default();
        let (mut cov, warns) = yavs_cov::collect(&db, &e, &counts, a.cov_exclude_translate_off);
        for w in &warns {
            out.emit(&none, w);
        }
        cov.items.retain(|k, _| match k.kind {
            yavs_cov::Kind::Statement => kind(CovKindArg::Statement),
            yavs_cov::Kind::Branch(_) => kind(CovKindArg::Branch),
            yavs_cov::Kind::Toggle(_) | yavs_cov::Kind::Fsm(_) => false,
            yavs_cov::Kind::Condition(_) => kind(CovKindArg::Condition) || kind(CovKindArg::Assert),
            yavs_cov::Kind::Expression(_) => kind(CovKindArg::Expression),
            yavs_cov::Kind::Combination(_) => kind(CovKindArg::Combination),
        });
        if let Some(t) = &toggles {
            t.add_to(&mut cov);
        }
        if let Some(f) = &fsm {
            f.add_to(&mut cov);
        }
        cov.unmeasured = unmeasured;
        cov.attribute(
            &a.cov_test
                .clone()
                .unwrap_or_else(|| test_name(&a.common.top, &a.common.generics)),
        );
        if a.cov_embed_sources {
            cov.embed_sources(&db);
        }
        if let Err(err) = std::fs::write(path, cov.write()) {
            out.emit(
                &none,
                &Diagnostic::error("E0110", format!("cannot write coverage database `{path}`"))
                    .with_note(err.to_string()),
            );
            code = code.max(1);
        }
    }
    if tokens.is_some_and(|p| p.lost()) {
        out.emit(
            &none,
            &token_gone("the server closed the connection during the run".into()),
        );
    }
    let code = if out.errors > 0 { code.max(1) } else { code };
    let code = if crate::stdout::is_broken() {
        crate::stdout::BROKEN_STATUS
    } else {
        code
    };
    let done = out.finish(code);
    // The optimised copies of this run's functions go into the stored lowering of the design.
    e.learn();
    if let Some(t) = tier_up {
        // `end` may end the process at once, without destructors: the files the design left
        // open are written out and closed first.
        if let Some(rt) = &rt {
            rt.close_files();
        }
        t.end(code, !a.common.load.is_empty() || !a.vhpi.is_empty());
    }
    // The process ends now: freeing the design and the runtime piece by piece only hands memory
    // back to an operating system that takes it anyway (a tenth of the run for a large design).
    // Runs with a waveform file or a plugin keep their orderly teardown. The files the design
    // left open are written out and closed first.
    if a.wave.is_none() && a.vhpi.is_empty() {
        if let Some(rt) = &rt {
            rt.close_files();
        }
        std::mem::forget((sim, built, rt, e, db));
    }
    done
}

/// The seed of a shuffled run, once at the start and again in a diagnostic that ends or
/// explains a run, so a failure found in a long log can be replayed.
fn shuffle_note(seed: u64) -> Diagnostic {
    Diagnostic::new(
        yavs_diag::Severity::Note,
        "",
        format!("process order is shuffled with seed {seed}; replay it with `--shuffle={seed}`"),
    )
}

fn with_seed(d: Diagnostic, shuffle: Option<u64>) -> Diagnostic {
    match shuffle {
        Some(s) => d.with_note(format!(
            "process order was shuffled with seed {s}; replay it with `--shuffle={s}`"
        )),
        None => d,
    }
}

fn wave_error(path: &str, err: yavs_wave::Error) -> Diagnostic {
    Diagnostic::error("E0801", format!("cannot write the waveform file `{path}`"))
        .with_note(err.to_string())
}

/// Ends the process from the signal thread. With the compiled tier built in, `exit` would run
/// the static destructors of LLVM under a compile thread that is inside LLVM (a crash, #625), so
/// the process ends without exit handlers, unless `keep_exit` (foreign libraries or VHPI plugins
/// loaded, whose exit handlers must run).
fn exit_after_signal(code: u8, keep_exit: bool) -> ! {
    #[cfg(feature = "llvm")]
    if !keep_exit {
        yavs_llvm::exit_now(code);
    }
    let _ = keep_exit;
    std::process::exit(code.into())
}

/// `--exit-on-stdin-close`: standard input goes to the null device for everything else (a plugin
/// or foreign library must not touch the pipe, see `yavs_vhpi::divert_stdin`) and a thread reads
/// the real one to its end, then ends the process as a signal would (status 130). Whatever
/// arrives on the way is dropped.
// ponytail: no orderly stop (wave and coverage files stay unfinished); route it through
// `SimControl::stop` like Ctrl-C if a parent that dies mid-run should leave them complete.
fn exit_when_stdin_ends(keep_exit: bool) -> Result<(), std::io::Error> {
    let mut stdin = yavs_vhpi::divert_stdin()?;
    std::thread::Builder::new()
        .name("stdin-watch".into())
        .spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf) {
                    Ok(n) if n > 0 => {}
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    _ => break,
                }
            }
            crate::stdout::flush();
            let _ = writeln!(
                std::io::stderr(),
                "yavs: standard input closed (the parent ended): stopping the run"
            );
            exit_after_signal(130, keep_exit)
        })?;
    Ok(())
}

/// The first Ctrl-C/SIGTERM sets `flag` and ends the run between cycles; the second writes out
/// the buffered output and exits.
fn ctrl_c_stops(
    hooks: &mut yavs_wave::interrupt::Handlers,
    stop: impl Fn() + Send + 'static,
    flag: Arc<AtomicBool>,
    keep_exit: bool,
) {
    hooks.first = Some(Box::new(move || {
        flag.store(true, Ordering::SeqCst);
        stop();
    }));
    hooks.at_exit = Some(Box::new(move || {
        crate::stdout::flush();
        exit_after_signal(1, keep_exit);
    }));
}

fn handler_error(err: std::io::Error) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error("E0803", "cannot install the Ctrl-C handler").with_note(err.to_string()),
    )
}

/// Where the wave layout of this testbench is kept: in the work library, by the name of the top.
fn layout_path(a: &RunArgs) -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(a.common.work.split_once(':')?.1);
    let top = a.common.top.to_ascii_lowercase();
    Some(dir.join("yavs-wave-layout").join(format!("{top}.txt")))
}

/// The `--surfer` viewer of a run, made with its first recorder.
type ViewSlot = Arc<std::sync::Mutex<Option<Arc<crate::surfer::Surfer>>>>;

/// Show the whole run in the viewer. Lost contact is a warning that `-W error` does not promote
/// (user, 2026-10-30): the viewer never changes the status of a run.
fn view_end(view: &ViewSlot, format: MessageFormat, color: bool) {
    let v = view.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(w) = v.and_then(|v| v.end()) {
        crate::stdout::flush();
        eprint!(
            "{}",
            yavs_diag::render(&SourceMap::default(), &w, format, color)
        );
    }
}

/// Subscribe the VHPI observer (`--vhpi`) and the waveform recorder (`--wave`) to `sim`, for
/// `yavs run` with or without `--dap`; returns the recorder and its file. `--surfer` starts the
/// viewer with the first recorder, and tells it of the next ones.
fn observers(
    a: &RunArgs,
    db: &yavs_sem::Db,
    e: &yavs_elab::Elaborated,
    sim: &mut yavs_kernel::SimControl,
    built: Option<&yavs_elab::Built>,
    out: &mut Out,
    view: &ViewSlot,
) -> Result<Option<(String, yavs_wave::Recorder)>, Box<Diagnostic>> {
    let none = SourceMap::default();
    if !a.vhpi.is_empty() {
        if let Some(b) = built.cloned() {
            yavs_vhpi::set_runtime(b);
        }
        sim.subscribe(Box::new(yavs_vhpi::VhpiObserver));
    }
    let recorder = match &a.wave {
        None => None,
        Some(path) => {
            let sel = yavs_wave::Selection {
                include: a
                    .wave_include
                    .iter()
                    .map(|p| yavs_diag::dotted(p))
                    .collect(),
                exclude: a
                    .wave_exclude
                    .iter()
                    .map(|p| yavs_diag::dotted(p))
                    .collect(),
                max_array_elems: a.wave_max_array,
            };
            let mut cfg = yavs_wave::WaveConfig::default();
            // `--surfer` flushes every second unless `--surfer-refresh` or the variable say
            // otherwise (#876).
            if a.surfer.is_some() {
                cfg.flush_interval = Some(std::time::Duration::from_secs(1));
            }
            // Default: no flush by the clock (reproducible bytes); a duration opts in (#666).
            match std::env::var("YAVS_WAVE_FLUSH").as_deref() {
                Err(_) => {}
                Ok("never") => cfg.flush_interval = None,
                Ok(v) => match parse_flush_interval(v) {
                    Some(d) => cfg.flush_interval = Some(d),
                    None => {
                        return Err(Box::new(
                            Diagnostic::error(
                                "E0801",
                                format!("cannot write the waveform file `{path}`"),
                            )
                            .with_note(format!(
                                "`YAVS_WAVE_FLUSH={v}`: use `never` or a duration such as `5s` or `500ms`"
                            )),
                        ));
                    }
                },
            }
            if a.surfer_refresh.is_some() {
                cfg.flush_interval = a.surfer_refresh;
            }
            let every = cfg.flush_interval;
            match yavs_wave::record(
                db,
                e,
                std::path::Path::new(path),
                sel,
                cfg,
                built.filter(|_| a.wave_vars()),
            ) {
                Ok((rec, hier)) => {
                    for s in &hier.skipped {
                        let why = match s.elems {
                            Some(n) => format!(
                                "it has {n} elements, more than `--wave-max-array={}`",
                                a.wave_max_array
                            ),
                            None => "its bounds are known only when the simulation runs".into(),
                        };
                        out.emit(
                            &none,
                            &Diagnostic::warning(
                                "W0802",
                                format!("`{}` is not recorded in the waveform", s.path),
                            )
                            .with_note(why),
                        );
                    }
                    sim.subscribe(Box::new(rec.clone()));
                    if let Some(addr) = &a.surfer {
                        // The file holds its hierarchy once a block is written.
                        rec.sync();
                        let mut slot = view.lock().unwrap_or_else(|p| p.into_inner());
                        match &*slot {
                            Some(v) => v.restart(),
                            None => {
                                if a.wave_default {
                                    let note = format!("the waveform is written to `{path}`");
                                    out.emit(
                                        &none,
                                        &Diagnostic::new(yavs_diag::Severity::Note, "", note),
                                    );
                                }
                                let all = !a.wave_include.is_empty();
                                let (mut names, over) = crate::surfer::names(&hier, all);
                                let recorded = crate::surfer::recorded(&hier);
                                // The list of the last session of this testbench, what still exists.
                                if let Some(saved) = layout_path(a).and_then(|f| {
                                    yavs_dap::load_layout(&f, &|i| {
                                        recorded.contains_key(&i.to_ascii_lowercase())
                                    })
                                    .ok()
                                }) {
                                    names = saved.0;
                                }
                                if over > 0 {
                                    let note = format!(
                                        "{over} more signals are recorded but not shown: `wave add` or Surfer's own commands add them"
                                    );
                                    out.emit(
                                        &none,
                                        &Diagnostic::new(yavs_diag::Severity::Note, "", note),
                                    );
                                }
                                *slot = Some(crate::surfer::Surfer::start(
                                    addr.as_deref(),
                                    std::path::Path::new(path),
                                    names,
                                    recorded,
                                    every,
                                    layout_path(a),
                                )?);
                            }
                        }
                    }
                    Some((path.clone(), rec))
                }
                Err(err) => return Err(Box::new(wave_error(path, err))),
            }
        }
    };
    Ok(recorder)
}

/// Prints reports and `OUTPUT` text in sim1's format on stdout (cli-contract section 5;
/// tools/difftest parses it).
struct Printer;

impl yavs_kernel::Observer for Printer {
    fn wants_phase(&self, _: yavs_kernel::Phase) -> bool {
        false
    }

    fn wants_value_changes(&self) -> bool {
        false
    }

    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        let at = match (r.process, &r.context) {
            (Some(p), _) => format!("Process {}", hook.process_name(p).unwrap_or("?")),
            (None, c) => format!("Function {}", c.as_deref().unwrap_or("?")),
        };
        print_report(hook.now(), hook.delta(), r.severity, &r.message, &at);
    }

    fn output(&mut self, _: &mut yavs_kernel::Hook<'_>, _: yavs_kernel::ProcId, bytes: &[u8]) {
        crate::stdout::write(bytes);
    }

    // Processes that run serially write their reports as they make them (#483).
    fn live_sink(&self) -> Option<Arc<dyn yavs_kernel::LiveSink>> {
        Some(Arc::new(Printer))
    }
}

impl yavs_kernel::LiveSink for Printer {
    fn report(
        &self,
        now: yavs_kernel::Time,
        delta: u32,
        process: &str,
        severity: yavs_kernel::Severity,
        message: &str,
    ) {
        print_report(now, delta, severity, message, &format!("Process {process}"));
    }

    fn output(&self, bytes: &[u8]) {
        crate::stdout::write(bytes);
    }
}

fn print_report(
    now: yavs_kernel::Time,
    delta: u32,
    severity: yavs_kernel::Severity,
    message: &str,
    at: &str,
) {
    let sev = match severity {
        yavs_kernel::Severity::Note => "Note",
        yavs_kernel::Severity::Warning => "Warning",
        yavs_kernel::Severity::Error => "Error",
        yavs_kernel::Severity::Failure => "Failure",
    };
    let text = format!("** {sev}: {}fs+{delta}: {message}\n    {at}\n", now.as_fs());
    crate::stdout::write(text.as_bytes());
}

/// The default `--cov-test`: the enabled test cases of a `runner_cfg` generic (its
/// `enabled_test_cases : NAME` field; `,,` is a comma), else `top(g1=v1,g2=v2)` with the
/// `-g` generics in command-line order.
fn test_name(top: &str, generics: &[(String, String)]) -> String {
    let cfg = generics
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("runner_cfg"))
        .and_then(|(_, v)| {
            let v = v.trim_matches('"').replace(",,", "\u{0}");
            v.split(',').find_map(|f| {
                let (k, val) = f.split_once(':')?;
                (k.trim() == "enabled_test_cases").then(|| val.trim().replace('\u{0}', ","))
            })
        })
        .filter(|t| !t.is_empty());
    if let Some(t) = cfg {
        return t;
    }
    if generics.is_empty() {
        return top.to_string();
    }
    let g: Vec<String> = generics.iter().map(|(n, v)| format!("{n}={v}")).collect();
    format!("{top}({})", g.join(","))
}

#[test]
fn default_test_names() {
    let g = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    };
    assert_eq!(test_name("tb", &[]), "tb");
    assert_eq!(
        test_name("tb", &g(&[("n", "0"), ("w", "8")])),
        "tb(n=0,w=8)"
    );
    let cfg = "\"enabled_test_cases : lib.tb.a,,b,output path : x,\"";
    assert_eq!(test_name("tb", &g(&[("runner_cfg", cfg)])), "lib.tb.a,b");
}

/// `10ms`, `1.5 ns`: a TIME literal.
/// The help line for the unit of a time `t` that names none: `10nss` has a unit like `ns`.
fn time_unit_help(t: &str) -> Option<String> {
    let t = t.trim_end();
    let unit = t.trim_start_matches(|c: char| !c.is_ascii_alphabetic());
    let units = ["fs", "ps", "ns", "us", "ms", "sec", "min", "hr"];
    let cands = units.into_iter().map(|u| (u, u));
    yavs_diag::suggest::suggest(&unit.to_ascii_lowercase(), "time unit", cands)
}

fn parse_time(t: &str) -> Result<yavs_kernel::Time, String> {
    let fs = yavs_std::value::time_literal(t).map_err(|e| e.to_string())?;
    yavs_kernel::Time::from_vhdl(fs).map_err(|e| e.to_string())
}

#[test]
fn gc_threshold_sizes() {
    assert_eq!(parse_size("4096"), Ok(4096));
    assert_eq!(parse_size("64k"), Ok(64 << 10));
    assert_eq!(parse_size("16M"), Ok(16 << 20));
    assert_eq!(parse_size("1g"), Ok(1 << 30));
    for bad in [
        "0",
        "0m",
        "",
        "m",
        "-1",
        "1.5m",
        "4x",
        "99999999999g",
        "1 m",
    ] {
        assert!(parse_size(bad).is_err(), "{bad}");
    }
}
