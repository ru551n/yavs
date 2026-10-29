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

use std::io::Write;

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
    /// Neither read nor write the compilation cache of the work library.
    #[arg(long)]
    no_cache: bool,
    /// Directory of the std/ieee units shared by all libraries, or `off` (default: `$YAVS_STD_CACHE`,
    /// else `yavs/std` in the user's cache directory).
    #[arg(long, value_name = "DIR|off")]
    std_cache: Option<std::ffi::OsString>,
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
    /// Write an FST of the whole design.
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
    /// Also record the scalar variables of processes (sampled at the end of each time step);
    /// the include and exclude globs apply to `top.proc.var`.
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
    /// Stop the simulation at this time, e.g. `10ms`.
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
    /// Worker threads for process evaluation (at least 1).
    #[arg(long, default_value = "1", value_parser = parse_threads)]
    threads: usize,
    /// Collect the heap of access values once this much was allocated since the last
    /// collection (and as much as that collection kept): plain bytes or a `k`, `m` or `g`
    /// suffix (powers of 1024), at least 1. Default 1m.
    #[arg(long, value_name = "SIZE", value_parser = parse_size)]
    gc_threshold: Option<u64>,
    /// Randomize process execution order, with an optional seed.
    #[arg(long, value_name = "SEED", num_args = 0..=1, require_equals = true, default_missing_value = "1")]
    shuffle: Option<u64>,
    /// Count statement, branch and toggle coverage and write the run's coverage database to
    /// FILE (default `yavs.ycov`).
    #[arg(long, value_name = "FILE", num_args = 0..=1, require_equals = true, default_missing_value = "yavs.ycov")]
    cov: Option<String>,
    /// With `--cov`: the kinds of coverage to count, comma separated.
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_value = "statement,branch,toggle"
    )]
    cov_kind: Vec<CovKindArg>,
    /// With `--cov`: the name of this run's test in the database (default: the enabled test
    /// case of a `runner_cfg` generic, else the top level with its `-g` generics).
    #[arg(long, value_name = "NAME")]
    cov_test: Option<String>,
    /// With `--cov`: leave signals with more `bit`/`std_ulogic` scalars than this out of
    /// toggle coverage.
    #[arg(long, value_name = "N", default_value = "65536")]
    cov_toggle_max: u64,
    /// With `--cov`: also leave `translate_off` ... `translate_on` regions out of the coverage.
    #[arg(long)]
    cov_exclude_translate_off: bool,
    /// Load a shared library for foreign subprograms.
    #[arg(long, value_name = "LIB")]
    load: Vec<String>,
    /// Load a VHPI plugin, `LIB[:ENTRY]`.
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
    /// on standard input and output (`stdio`).
    #[arg(long, value_name = "PORT|stdio", value_parser = yavs_dap::parse_transport)]
    dap: Option<yavs_dap::Transport>,
    /// Open the run in the graphical debugger: starts `yavs-gui` (next to this executable, else
    /// on PATH) with these run arguments.
    #[arg(long, conflicts_with = "dap")]
    pub(crate) gui: bool,
    /// Plusargs `+NAME[=VALUE]` after the top level, seen by VHPI plugins as tool arguments.
    #[arg(value_name = "+NAME[=VALUE]", value_parser = plusarg)]
    plusargs: Vec<String>,
}

/// The compiled tiers of this build, for sentences like "this yavs has ...".
pub(crate) fn tiers_built() -> &'static str {
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
         in the build: this yavs has {built}. Default: `auto` with a compiled tier, `interp` with \
         `--dap` and without one."
    )
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
        Err(e) => Err(format!("`{s}` is not a number of threads: {e}")),
    }
}

fn parse_max_deltas(s: &str) -> Result<u32, String> {
    match s.trim().parse::<u32>() {
        Ok(0) => Err("the limit must be at least 1".into()),
        Ok(n) => Ok(n),
        Err(e) => Err(format!("`{s}` is not a number of delta cycles: {e}")),
    }
}

fn heat(s: &str) -> Result<[u32; 3], String> {
    let v: Vec<u32> = s
        .split(',')
        .map(|v| v.trim().parse::<u32>().map_err(|e| format!("`{v}`: {e}")))
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

impl RunArgs {
    /// The tier of this run: the one asked for, else `auto` where it is built in and nothing
    /// needs the interpreter alone (a debugger reads frame slots).
    pub(crate) fn tier(&self) -> TierArg {
        let compiled = cfg!(feature = "llvm") || cfg!(feature = "clift");
        let t = self.tier.unwrap_or(if compiled && self.dap.is_none() {
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
    Interp,
    Llvm,
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
    Statement,
    Branch,
    Toggle,
    Condition,
    Expression,
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

/// Loads the `--load` libraries and binds every foreign subprogram of the program to its
/// symbol: in the libraries in order, then in the executable (ir.md 1.7).
fn bind_foreign(
    load: &[String],
    e: &mut yavs_elab::Elaborated,
    out: &mut Out,
) -> Result<(), Box<Diagnostic>> {
    let Some(ir) = e.ir.as_mut() else {
        return Ok(());
    };
    let mut libs = Vec::new();
    for l in load {
        let lib = yavs_vhpi::load_foreign(l.as_ref()).map_err(|err| {
            let d = Diagnostic::error("E0106", format!("cannot load library `{l}` (`--load`)"));
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
        match yavs_vhpi::bind_foreign(&libs, &x.symbol, x.signature()) {
            Some(Ok(f)) => ir.foreign.push((id, f)),
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

fn print_hierarchy(db: &Db, e: &yavs_elab::Elaborated) {
    use yavs_elab::Case;
    let mut s = String::new();
    for (i, sc) in e.hier.scopes.iter().enumerate() {
        let id = yavs_elab::ScopeId(i as u32);
        if matches!(sc.kind, yavs_elab::ScopeKind::Component { .. }) {
            continue;
        }
        s.push_str(&e.hier.instance_name(id, db, Case::Original));
        s.push('\n');
        for &o in &sc.objs {
            let l = e.nets.obj(o);
            s.push_str(&format!(
                "{}{} ({:?})\n",
                e.hier.path_name(id, Case::Original),
                l.name.text,
                l.kind
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
        let tier = yavs_clift::compile(&rt.prog, a.tier_max_size).map_err(bug)?;
        if a.verbose {
            for (f, why) in &tier.notes {
                let name = rt.prog.func(*f).map_or("?", |f| f.name.as_str());
                eprintln!("note: `{name}` ({f}) stays interpreted under --tier=clift: {why}");
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
                "note: `{name}` ({f}, {n} IR instructions) is hot: compiled by --tier=clift-auto in {} us",
                t.as_micros()
            ),
            Err(why) => eprintln!("note: `{name}` ({f}) is hot but stays interpreted: {why}"),
        }
    });
    let first = yavs_clift::First::new().map_err(bug)?;
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
                "note: `{name}` ({f}, {n} IR instructions) is hot: compiled by --tier=auto3 with {by} in {} us",
                t.as_micros()
            ),
            Err(why) => eprintln!("note: `{name}` ({f}) is hot but stays interpreted: {why}"),
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
                        "note: `{name}` ({f}, {n} IR instructions) is hot: compiled by --tier=auto at {level:?} in {} ms",
                        t.as_millis()
                    ),
                    Err(why) => {
                        eprintln!("note: `{name}` ({f}) is hot but stays interpreted: {why}")
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
            eprintln!("note: `{name}` ({f}) stays interpreted under --tier=llvm: {why}");
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

pub fn run(
    a: RunArgs,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    // First: a descriptor of the token pool the parent closed may be reused by a file below.
    let tokens = match (a.threads > 1 && a.dap.is_none()).then(crate::jobserver::connect) {
        Some(Err(why)) => {
            let mut out = Out::new(format, color);
            out.emit(
                &SourceMap::default(),
                &Diagnostic::error("E0128", "the token pool in `MAKEFLAGS` cannot be used")
                    .with_note(why)
                    .with_help(
                        "pass the pool's descriptors or named pipe on to yavs; a parent that does \
                         not share its pool with this run removes `--jobserver-auth` from \
                         `MAKEFLAGS` for it",
                    ),
            );
            return out.finish(2);
        }
        Some(Ok(p)) => p,
        None => None,
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
    if a.dap.is_some() {
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
    if !a.vhpi.is_empty() || a.wave_variables {
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
    let lowering_counters = kind(CovKindArg::Statement)
        || kind(CovKindArg::Branch)
        || kind(CovKindArg::Condition)
        || kind(CovKindArg::Expression);
    let lowering = yavs_elab::LowerOptions {
        coverage: kind(CovKindArg::Statement) || kind(CovKindArg::Branch),
        cov_condition: kind(CovKindArg::Condition),
        cov_expression: kind(CovKindArg::Expression),
        ..yavs_elab::LowerOptions::default()
    };
    // A debugger or VHPI plugin may force or deposit any port: those runs keep ports apart.
    let collapse = a.dap.is_none() && a.vhpi.is_empty();
    let (mut out, mut db, e) = match elaborate(
        &a.common,
        lowering,
        matches!(a.tier(), TierArg::Llvm | TierArg::Clift),
        collapse,
        format,
        color,
        encoding,
    ) {
        Ok(x) => x,
        Err((out, code)) => return out.finish(code),
    };
    if a.common.print_hierarchy {
        print_hierarchy(&db, &e);
    }
    let none = SourceMap::default();
    let unsupported = |what: &str| {
        Diagnostic::error("E9999", format!("{what} is not implemented yet in yavs"))
            .with_note("this is a limitation of yavs, not an error in the design")
    };
    if a.dap.is_some() {
        // A debugger session runs the interpreter whatever `--tier` says, but a tier this build
        // lacks is E0120 as without `--dap` (#835): nothing is compiled, so no runtime is given.
        if let Err(d) = use_tier(None, &a) {
            out.emit(&none, &d);
            return out.finish(2);
        }
        if a.profile.is_some() {
            out.emit(
                &none,
                &Diagnostic::error("E0125", "`--profile` cannot be combined with `--dap`")
                    .with_note(
                        "a stop at a breakpoint would be sampled as time in the stopped process",
                    )
                    .with_help("profile a run without `--dap`"),
            );
            return out.finish(2);
        }
        if a.cov.is_some() {
            out.emit(&none, &unsupported("`--cov` together with `--dap` (M8)"));
            return out.finish(2);
        }
        if a.threads > 1 {
            out.emit(
                &none,
                &Diagnostic::warning("W0109", "`--threads` is ignored with `--dap`")
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
        for spec in &a.vhpi {
            let p: yavs_vhpi::PluginSpec = spec.parse().unwrap_or_else(|e| match e {});
            let loaded = yavs_vhpi::load(&p).map_err(|err| err.to_diagnostic());
            if let Err(d) = loaded {
                out.emit(&none, &d);
                return out.finish(2);
            }
        }
    }
    // The models of VHPI foreign architectures run now, before the build (#330).
    let mut e = e;
    if !e.foreign_archs.is_empty() {
        match yavs_vhpi::elaborate_foreign(&e) {
            Ok(procs) => e.ext_procs = procs,
            Err(d) => {
                out.emit(&none, &d);
                return out.finish(2);
            }
        }
    }
    let others_free = a.dap.is_none() && a.vhpi.is_empty();
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
        edge_triggers: a.dap.is_none() && a.vhpi.is_empty(),
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
    };
    if let Some(seed) = a.shuffle {
        out.emit(&none, &shuffle_note(seed));
    }
    let cfg_profile = a
        .profile
        .is_some()
        .then(yavs_kernel::profile::Profile::install);
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
                        d.with_note(m)
                    },
                );
                return out.finish(2);
            }
        },
    };
    let mut e = e;
    let bound = bind_foreign(&a.load, &mut e, &mut out);
    if let Err(d) = bound {
        out.emit(&none, &d);
    }
    if out.errors > 0 {
        return out.finish(2);
    }
    if let Some(d) = yavs_elab::check_time_resolution(&db, cfg.resolution) {
        out.emit(&db.sources, &d);
        return out.finish(2);
    }
    if let Some(t) = a.dap {
        let (db, e) = (Arc::new(db), Arc::new(e));
        let attach = |sim: &mut yavs_kernel::SimControl, built: Option<&yavs_elab::Built>| {
            let rec = observers(&a, &db, &e, sim, built, &mut out)?;
            if rec.is_some() {
                // Ctrl-C ends the run between cycles so the file is finished (rule 11).
                let mut hooks = yavs_wave::interrupt::Handlers::default();
                ctrl_c_stops(&mut hooks, sim.interrupt(), Arc::default(), true);
                yavs_wave::interrupt::start(hooks).map_err(handler_error)?;
            }
            Ok(rec.map(|(path, rec)| {
                let r = rec.clone();
                let finish: yavs_dap::Finish =
                    Box::new(move |sim| rec.finish(sim).err().map(|err| wave_error(&path, err)));
                let sync: yavs_dap::Sync = Box::new(move || r.sync());
                (finish, sync)
            }))
        };
        let report: yavs_dap::Report = Box::new(|stats, rt| {
            crate::counters::report(stats, rt);
            yavs_interp::census_dump(40);
        });
        let r = yavs_dap::serve(db.clone(), e.clone(), cfg, stop, t, attach, report);
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
    let (mut sim, built) = match e.build_full_once(cfg) {
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
    let tier_up = match use_tier(built.as_ref().map(|b| &*b.rt), &a) {
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
    if a.vhpi.is_empty() && a.load.is_empty() && !foreign {
        crate::stdout::buffer();
    }
    if let Some(p) = tokens {
        sim.set_tokens(p);
    }
    sim.subscribe(Box::new(Printer));
    let on_pipe = sim.interrupt();
    crate::stdout::on_broken(move || on_pipe.stop());
    let rt = built.as_ref().map(|b| b.rt.clone());
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
    let interrupted = Arc::new(AtomicBool::new(false));
    let recorder = match observers(&a, &db, &e, &mut sim, built.as_ref(), &mut out) {
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
    let mut hooks = yavs_wave::interrupt::Handlers {
        usr1: Some(crate::stuck::on_request(sim.interrupt())),
        ..Default::default()
    };
    if crate::stdout::is_on() {
        hooks.tick = Some((crate::stdout::TICK, Box::new(crate::stdout::flush)));
    }
    let launch = tier_up.as_ref().map(|t| t.launcher(format, color));
    hooks.after = launch
        .clone()
        .map(|(after, f)| (after, Box::new(move || f()) as _));
    let keep_exit = !a.load.is_empty() || !a.vhpi.is_empty();
    if recorder.is_some() || cfg_profile.is_some() {
        let flag = if recorder.is_some() {
            &interrupted
        } else {
            &signalled
        };
        ctrl_c_stops(&mut hooks, sim.interrupt(), flag.clone(), keep_exit);
    } else if crate::stdout::is_on() {
        hooks.first = Some(Box::new(move || {
            crate::stdout::flush();
            exit_after_signal(130, keep_exit);
        }));
    } else if crate::jobserver::active() {
        // The default action would keep the tokens the kernel holds.
        hooks.first = Some(Box::new(move || exit_after_signal(130, keep_exit)));
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
        )
    });
    let max = a.hang_report.unwrap_or(30);
    let r = loop {
        let r = sim.run_until(stop);
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
            yavs_cov::Kind::Toggle(_) => false,
            yavs_cov::Kind::Condition(_) => kind(CovKindArg::Condition),
            yavs_cov::Kind::Expression(_) => kind(CovKindArg::Expression),
        });
        if let Some(t) = &toggles {
            t.add_to(&mut cov);
        }
        cov.attribute(
            &a.cov_test
                .clone()
                .unwrap_or_else(|| test_name(&a.common.top, &a.common.generics)),
        );
        if let Err(err) = std::fs::write(path, cov.write()) {
            out.emit(
                &none,
                &Diagnostic::error("E0110", format!("cannot write coverage database `{path}`"))
                    .with_note(err.to_string()),
            );
            code = code.max(1);
        }
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
        t.end(code, !a.load.is_empty() || !a.vhpi.is_empty());
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
    crate::jobserver::close();
    #[cfg(feature = "llvm")]
    if !keep_exit {
        yavs_llvm::exit_now(code);
    }
    let _ = keep_exit;
    std::process::exit(code.into())
}

/// The first Ctrl-C/SIGTERM sets `flag` and ends the run between cycles; the second writes out
/// the buffered output and exits.
fn ctrl_c_stops(
    hooks: &mut yavs_wave::interrupt::Handlers,
    stop: yavs_kernel::Interrupt,
    flag: Arc<AtomicBool>,
    keep_exit: bool,
) {
    hooks.first = Some(Box::new(move || {
        flag.store(true, Ordering::SeqCst);
        stop.stop();
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

/// Subscribe the VHPI observer (`--vhpi`) and the waveform recorder (`--wave`) to `sim`, for
/// `yavs run` with or without `--dap`; returns the recorder and its file.
fn observers(
    a: &RunArgs,
    db: &yavs_sem::Db,
    e: &yavs_elab::Elaborated,
    sim: &mut yavs_kernel::SimControl,
    built: Option<&yavs_elab::Built>,
    out: &mut Out,
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
                include: a.wave_include.clone(),
                exclude: a.wave_exclude.clone(),
                max_array_elems: a.wave_max_array,
            };
            let mut cfg = yavs_wave::WaveConfig::default();
            // Default: no flush by the clock (reproducible bytes); a duration opts in (#666).
            match std::env::var("YAVS_WAVE_FLUSH").as_deref() {
                Err(_) | Ok("never") => {}
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
            match yavs_wave::record(
                db,
                e,
                std::path::Path::new(path),
                sel,
                cfg,
                built.filter(|_| a.wave_variables),
            ) {
                Ok((rec, skipped)) => {
                    for s in skipped {
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
