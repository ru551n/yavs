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
    /// Relax some LRM checks (sem.md 8.4).
    #[arg(long)]
    relaxed: bool,
    /// Read `-- psl` comments as PSL pragmas (native PSL statements are always read).
    #[arg(long)]
    psl: bool,
    /// Warning control: `-Werror`, `-Werror=CODE`, `-Wno-CODE`, `-WCODE` (docs/cli-contract.md 4).
    #[arg(short = 'W', value_name = "FLAG")]
    warn: Vec<String>,
    /// Neither read nor write the compilation cache of the work library.
    #[arg(long)]
    no_cache: bool,
    /// Keep every `in` port its own signal instead of an alias of its actual (docs/cli-contract.md).
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

#[derive(Clone, Copy, clap::ValueEnum)]
enum ResolutionArg {
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    Sec,
}

#[derive(ClapArgs)]
pub struct RunArgs {
    #[command(flatten)]
    common: Common,
    /// A report at or above this severity stops the simulation with exit 1; an `error`
    /// report below it is printed, the run continues and exits 1 at the end.
    #[arg(long, value_enum, default_value = "failure")]
    exit_severity: SeverityArg,
    /// Suppress warnings raised inside `ieee` packages.
    #[arg(long, value_name = "on|off")]
    ieee_warnings: Option<String>,
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
    /// Stop the simulation at this time, e.g. `10ms`.
    #[arg(long, value_name = "TIME")]
    stop_time: Option<String>,
    /// Time resolution limit (`std.env.resolution_limit`, TIME values in reflection).
    #[arg(long, value_enum, default_value = "fs")]
    resolution: ResolutionArg,
    /// Worker threads for process evaluation.
    #[arg(long, default_value = "1")]
    threads: usize,
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
    /// Execution tier: `interp`; `llvm` to compile every function with LLVM before time 0; or
    /// `auto` to compile the functions that become hot on a background thread while the
    /// simulation runs (both need a yavs built with the `llvm` feature).
    #[arg(long, value_enum, default_value = "interp")]
    tier: TierArg,
    /// With `--tier=llvm` or `auto`: functions of more than N IR instructions stay
    /// interpreted (compile time grows faster than their size).
    #[arg(long, value_name = "N", default_value = "50000")]
    tier_max_size: usize,
    /// With `--tier=auto`: compile a function once the sampler (one sample every PERIOD
    /// microseconds) has seen the interpreter in it for BASE microseconds plus PER_INSTR
    /// nanoseconds per IR instruction it has; PERIOD 0: compile every function the interpreter
    /// enters (for tests and tuning).
    #[arg(long, value_name = "BASE,PER_INSTR,PERIOD", value_parser = heat, hide = true)]
    tier_heat: Option<[u32; 3]>,
    /// Print notes on how the run is set up (functions the LLVM tier leaves interpreted or
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

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TierArg {
    Interp,
    Llvm,
    Auto,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CovKindArg {
    Statement,
    Branch,
    Toggle,
}

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
    coverage: bool,
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
    for r in &refs {
        match yavs_lib::closure(&libs, r) {
            Ok(f) => {
                files = Ok(f);
                break;
            }
            // The last "unknown unit" is reported, but never in place of a real failure of an
            // earlier try (a changed or unreadable source file, a cycle).
            Err(d) if matches!(&files, Err(None)) => files = Err(Some(d)),
            Err(d) if matches!(&files, Err(Some(p)) if p.code == "L0031") => files = Err(Some(d)),
            Err(_) => {}
        }
    }
    let files = match files {
        Ok(f) => f,
        Err(d) => {
            let d = d.unwrap_or_else(|| Box::new(Diagnostic::error("E0104", "no top level")));
            out.emit(&none, &d);
            return Err((out, 2));
        }
    };
    let rev = match c.std {
        Some(StdArg::V2008) => LangRev::Vhdl2008,
        Some(StdArg::V2019) => LangRev::Vhdl2019,
        None if files.iter().any(|f| f.std == "2019") => LangRev::Vhdl2019,
        None => LangRev::Vhdl2008,
    };
    let mut opts = Options::new(rev, yavs_sem::default_stdlib());
    opts.libraries = libs;
    opts.relaxed = c.relaxed;
    opts.psl = c.psl;
    opts.encoding = encoding;
    let work_dir = c.work.split_once(':').map_or("", |(_, d)| d);
    let (mut db, analyzed) =
        crate::cache::analyze_closure(opts, &files, work_dir.as_ref(), !c.no_cache);
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
            ..yavs_elab::ElabConfig::default()
        };
        yavs_elab::elaborate(&mut db, t, &c.generics, &cfg)
    });
    match r {
        Ok(mut e) => {
            for d in &e.warnings {
                out.emit(&db.sources, d);
            }
            let opts = yavs_elab::LowerOptions {
                coverage,
                ..yavs_elab::LowerOptions::default()
            };
            if let Err(ds) = e.lower_with(&mut db, opts) {
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
    match elaborate(&a, false, true, format, color, encoding) {
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
#[cfg(feature = "llvm")]
struct StopTierUp(std::sync::Arc<yavs_llvm::AutoTier>);

#[cfg(feature = "llvm")]
impl StopTierUp {
    /// Exit with `code` at once if the compile thread is still busy, so the run never waits
    /// for compilation; with `keep_exit` (foreign libraries or VHPI plugins loaded, whose exit
    /// handlers must run) wait for it instead.
    fn end(self, code: u8, keep_exit: bool) {
        if !self.0.stop(keep_exit) {
            yavs_llvm::exit_now(code);
        }
    }
}

#[cfg(feature = "llvm")]
impl Drop for StopTierUp {
    fn drop(&mut self) {
        self.0.stop(true);
    }
}

#[cfg(not(feature = "llvm"))]
struct StopTierUp;

#[cfg(not(feature = "llvm"))]
impl StopTierUp {
    fn end(self, _: u8, _: bool) {}
}

/// Set up `--tier=llvm` (compile the program now) or `--tier=auto` (start tier-up, stopped
/// when the returned guard drops).
#[cfg(feature = "llvm")]
fn use_tier(
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
    match a.tier {
        TierArg::Interp => Ok(None),
        TierArg::Llvm => {
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
            let t = yavs_llvm::AutoTier::start(
                rt,
                yavs_llvm::OptLevel::O0,
                a.tier_max_size,
                heat,
                note,
            )
            .map_err(bug)?;
            Ok(Some(StopTierUp(t)))
        }
    }
}

/// Run the compiled program from now on (`--tier=llvm`).
#[cfg(feature = "llvm")]
fn use_llvm(rt: &yavs_interp::Runtime, tier: yavs_llvm::LlvmTier, verbose: bool) {
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
            "note: --tier=llvm: {} instructions compiled, {n} executed by the interpreter ({})",
            tier.inline,
            kinds.join(", ")
        );
    }
    rt.set_tier(std::sync::Arc::new(tier));
}

#[cfg(not(feature = "llvm"))]
fn use_tier(
    _: Option<&yavs_interp::Runtime>,
    a: &RunArgs,
) -> Result<Option<StopTierUp>, Box<Diagnostic>> {
    let name = match a.tier {
        TierArg::Interp => return Ok(None),
        TierArg::Llvm => "llvm",
        TierArg::Auto => "auto",
    };
    Err(Box::new(
        Diagnostic::error(
            "E0120",
            format!("`--tier={name}` needs a yavs built with the LLVM tier"),
        )
        .with_note("this yavs was built without the `llvm` cargo feature")
        .with_help("use `--tier=interp`, or build yavs with `--features llvm` (needs LLVM 21)"),
    ))
}

pub fn run(
    a: RunArgs,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    if a.time_report {
        crate::timing::enable();
    }
    // A debugger reads and writes the variables of subprograms in their frame slots: keep them
    // there, before any function is optimised.
    if a.dap.is_some() {
        yavs_interp::keep_variables();
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
    let counters = kind(CovKindArg::Statement) || kind(CovKindArg::Branch);
    // A debugger or VHPI plugin may force or deposit any port: those runs keep ports apart.
    let collapse = a.dap.is_none() && a.vhpi.is_empty();
    let (mut out, mut db, e) =
        match elaborate(&a.common, counters, collapse, format, color, encoding) {
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
    let observed_free = a.cov.is_none() && a.dap.is_none() && a.vhpi.is_empty();
    let cfg = yavs_kernel::Config {
        threads: a.threads.max(1),
        shuffle: a.shuffle,
        resolution,
        stop_severity: match a.exit_severity {
            SeverityArg::Note => yavs_kernel::Severity::Note,
            SeverityArg::Warning => yavs_kernel::Severity::Warning,
            SeverityArg::Error => yavs_kernel::Severity::Error,
            SeverityArg::Failure => yavs_kernel::Severity::Failure,
        },
        // A process left suspended by its edge trigger would be visible to statement coverage,
        // a debugger stepping through it, and foreign callbacks.
        edge_triggers: observed_free,
        skip_unchanged: observed_free && !db.facts().reads_activity,
        ..Default::default()
    };
    let _ = &a.ieee_warnings; // ponytail: no ieee processes run yet; filter reports once they do
    let stop = match &a.stop_time {
        None => yavs_kernel::Time::MAX,
        Some(t) => match parse_time(t) {
            Ok(t) => t,
            Err(m) => {
                out.emit(
                    &none,
                    &Diagnostic::error("E0105", format!("invalid `--stop-time={t}`")).with_note(m),
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
        let interrupted = Arc::new(AtomicBool::new(false));
        let attach = |sim: &mut yavs_kernel::SimControl, built: Option<&yavs_elab::Built>| {
            let rec = observers(&a, &db, &e, sim, built, &interrupted, &mut out)?;
            Ok(rec.map(|(path, rec)| {
                let r = rec.clone();
                let finish: yavs_dap::Finish =
                    Box::new(move |sim| rec.finish(sim).err().map(|err| wave_error(&path, err)));
                let sync: yavs_dap::Sync = Box::new(move || r.sync());
                (finish, sync)
            }))
        };
        let r = yavs_dap::serve(db.clone(), e.clone(), cfg, stop, t, attach);
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
    // Held to the end of the run: `end` or a drop ends tier-up.
    let tier_up = match use_tier(built.as_ref().map(|b| &*b.rt), &a) {
        Ok(g) => g,
        Err(d) => {
            let code = if d.code == "E0120" { 2 } else { 3 };
            out.emit(&none, &d);
            return out.finish(code);
        }
    };
    sim.subscribe(Box::new(Printer));
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
    let recorder = match observers(
        &a,
        &db,
        &e,
        &mut sim,
        built.as_ref(),
        &interrupted,
        &mut out,
    ) {
        Ok(r) => r,
        Err(d) => {
            out.emit(&none, &d);
            return out.finish(2);
        }
    };
    // Nothing below reads the elaborated model but its IR (run-time diagnostics) unless a
    // plugin, the waveform recorder or coverage does (#421).
    if a.vhpi.is_empty() && a.wave.is_none() && a.cov.is_none() {
        e.release_for_run();
    }
    crate::timing::mark("init");
    let r = sim.run_until(stop);
    crate::timing::mark("simulate");
    crate::timing::report();
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
            out.emit(&db.sources, &e.runtime_diagnostic(&err));
            if err.exit_status() == 101 { 3 } else { 1 }
        }
    };
    let _ = o.flush();
    drop(o);
    let mut code = code;
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
    let failed = yavs_elab::elaboration_reports(Error) + yavs_elab::elaboration_reports(Failure)
        > 0
        || rt
            .as_ref()
            .is_some_and(|rt| rt.assert_count(Error) + rt.assert_count(Failure) > 0);
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

fn wave_error(path: &str, err: yavs_wave::Error) -> Diagnostic {
    Diagnostic::error("E0801", format!("cannot write the waveform file `{path}`"))
        .with_note(err.to_string())
}

/// Subscribe the VHPI observer (`--vhpi`) and the waveform recorder (`--wave`) to `sim`, for
/// `yavs run` with or without `--dap`; returns the recorder and its file.
fn observers(
    a: &RunArgs,
    db: &yavs_sem::Db,
    e: &yavs_elab::Elaborated,
    sim: &mut yavs_kernel::SimControl,
    built: Option<&yavs_elab::Built>,
    interrupted: &Arc<AtomicBool>,
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
            let cfg = yavs_wave::WaveConfig::default();
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
                    // Ctrl-C ends the run between cycles so the file is finished (rule 11).
                    let (stop, flag) = (sim.interrupt(), interrupted.clone());
                    if let Err(err) = yavs_wave::interrupt::install(move || {
                        flag.store(true, Ordering::Relaxed);
                        stop.stop();
                    }) {
                        return Err(Box::new(
                            Diagnostic::error("E0803", "cannot install the Ctrl-C handler")
                                .with_note(err.to_string()),
                        ));
                    }
                    Some((path.clone(), rec))
                }
                Err(err) => return Err(Box::new(wave_error(path, err))),
            }
        }
    };
    Ok(recorder)
}

/// Prints reports and `OUTPUT` text in NVC's format on stdout (cli-contract section 5;
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
        let sev = match r.severity {
            yavs_kernel::Severity::Note => "Note",
            yavs_kernel::Severity::Warning => "Warning",
            yavs_kernel::Severity::Error => "Error",
            yavs_kernel::Severity::Failure => "Failure",
        };
        let at = match (r.process, &r.context) {
            (Some(p), _) => format!("Process {}", hook.process_name(p).unwrap_or("?")),
            (None, c) => format!("Function {}", c.as_deref().unwrap_or("?")),
        };
        let mut o = std::io::stdout().lock();
        let _ = writeln!(
            o,
            "** {sev}: {}fs+{}: {}\n    {at}",
            hook.now().as_fs(),
            hook.delta(),
            r.message
        );
        let _ = o.flush();
    }

    fn output(&mut self, _: &mut yavs_kernel::Hook<'_>, _: yavs_kernel::ProcId, bytes: &[u8]) {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(bytes);
        let _ = o.flush();
    }
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
