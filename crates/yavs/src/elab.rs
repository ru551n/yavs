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
    /// Count statement and branch coverage and write the run's coverage database to FILE
    /// (default `yavs.ycov`).
    #[arg(long, value_name = "FILE", num_args = 0..=1, require_equals = true, default_missing_value = "yavs.ycov")]
    cov: Option<String>,
    /// With `--cov`: also leave `translate_off` ... `translate_on` regions out of the coverage.
    #[arg(long)]
    cov_exclude_translate_off: bool,
    /// Load a shared library for foreign subprograms.
    #[arg(long, value_name = "LIB")]
    load: Vec<String>,
    /// Load a VHPI plugin, `LIB[:ENTRY]`.
    #[arg(long, value_name = "LIB[:ENTRY]")]
    vhpi: Vec<String>,
    /// Execution tier: `interp`, or `llvm` to compile every function with LLVM before time 0
    /// (needs a yavs built with the `llvm` feature).
    #[arg(long, value_enum, default_value = "interp")]
    tier: TierArg,
    /// Print notes on how the run is set up (functions the LLVM tier leaves interpreted).
    #[arg(long)]
    verbose: bool,
    /// Run under a Debug Adapter Protocol client: on 127.0.0.1:PORT (0 picks a free port) or
    /// on standard input and output (`stdio`).
    #[arg(long, value_name = "PORT|stdio", value_parser = yavs_dap::parse_transport)]
    dap: Option<yavs_dap::Transport>,
    /// Plusargs `+NAME[=VALUE]` after the top level, seen by VHPI plugins as tool arguments.
    #[arg(value_name = "+NAME[=VALUE]", value_parser = plusarg)]
    plusargs: Vec<String>,
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
            Err(d) if files.is_err() => files = Err(Some(d)),
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
    let mut db = Db::new(opts);
    let analyzed = db.analyze_closure(&files);
    let diags = db.take_diagnostics();
    for d in &diags {
        out.emit(&db.sources, d);
    }
    if analyzed.is_err() || out.errors > 0 {
        return Err((out, 2));
    }
    let r = yavs_elab::find_top(&mut db, &work, &c.top).and_then(|t| {
        yavs_elab::elaborate(&mut db, t, &c.generics, &yavs_elab::ElabConfig::default())
    });
    match r {
        Ok(mut e) => {
            for d in &e.warnings {
                out.emit(&db.sources, d);
            }
            let opts = yavs_elab::LowerOptions { coverage };
            if let Err(ds) = e.lower_with(&mut db, opts) {
                for d in &ds {
                    out.emit(&db.sources, d);
                }
                return Err((out, 2));
            }
            if out.errors > 0 {
                return Err((out, 2));
            }
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
    match elaborate(&a, false, format, color, encoding) {
        Ok((out, db, e)) => {
            if a.print_hierarchy {
                print_hierarchy(&db, &e);
            }
            out.finish(0)
        }
        Err((out, code)) => out.finish(code),
    }
}

/// Compile the program with the LLVM tier and run it from now on (`--tier=llvm`); the error.
#[cfg(feature = "llvm")]
fn use_llvm(rt: Option<&yavs_interp::Runtime>, verbose: bool) -> Option<Diagnostic> {
    let rt = rt?;
    let tier = match yavs_llvm::compile(&rt.prog, yavs_llvm::OptLevel::O2) {
        Ok(t) => t,
        Err(e) => {
            return Some(
                Diagnostic::error("E0121", "the LLVM tier cannot compile the design")
                    .with_note(e.to_string())
                    .with_note("this is a bug in yavs"),
            );
        }
    };
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
    None
}

#[cfg(not(feature = "llvm"))]
fn use_llvm(_: Option<&yavs_interp::Runtime>, _: bool) -> Option<Diagnostic> {
    Some(
        Diagnostic::error(
            "E0120",
            "`--tier=llvm` needs a yavs built with the LLVM tier",
        )
        .with_note("this yavs was built without the `llvm` cargo feature")
        .with_help("use `--tier=interp`, or build yavs with `--features llvm` (needs LLVM 21)"),
    )
}

pub fn run(
    a: RunArgs,
    format: MessageFormat,
    color: bool,
    encoding: yavs_syntax::Encoding,
) -> ExitCode {
    let (mut out, db, e) = match elaborate(&a.common, a.cov.is_some(), format, color, encoding) {
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
        let with = if a.wave.is_some() {
            Some("`--wave`")
        } else if !a.vhpi.is_empty() {
            Some("`--vhpi`")
        } else if a.cov.is_some() {
            Some("`--cov`")
        } else {
            None
        };
        if let Some(w) = with {
            out.emit(
                &none,
                &unsupported(&format!("{w} together with `--dap` (M8)")),
            );
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
        return match yavs_dap::serve(&db, &e, cfg, stop, t) {
            Ok(code) => out.finish(code),
            Err(d) => {
                out.emit(&db.sources, &d);
                out.finish(2)
            }
        };
    }
    let probes = a.wave_variables.then(yavs_wave::VarProbes::default);
    let (mut sim, built) = match e.build_full(cfg, probes.as_ref()) {
        Ok(s) => s,
        // Package and instance initialisation runs VHDL code: report it with its VHDL stack.
        Err(err) => {
            out.emit(&db.sources, &e.runtime_diagnostic(&err));
            return out.finish(if err.exit_status() == 101 { 3 } else { 2 });
        }
    };
    if a.tier == TierArg::Llvm
        && let Some(d) = use_llvm(built.as_ref().map(|b| &*b.rt), a.verbose)
    {
        let code = if d.code == "E0120" { 2 } else { 3 };
        out.emit(&none, &d);
        return out.finish(code);
    }
    let names: Vec<String> = e.procs.iter().map(|p| e.proc_name(p)).collect();
    sim.subscribe(Box::new(Printer { names }));
    let rt = built.as_ref().map(|b| b.rt.clone());
    if !a.vhpi.is_empty() {
        if let Some(b) = built {
            yavs_vhpi::set_runtime(b);
        }
        sim.subscribe(Box::new(yavs_vhpi::VhpiObserver));
    }
    let wave_error = |path: &str, err: yavs_wave::Error| {
        Diagnostic::error("E0801", format!("cannot write the waveform file `{path}`"))
            .with_note(err.to_string())
    };
    let interrupted = Arc::new(AtomicBool::new(false));
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
                &db,
                &e,
                std::path::Path::new(path),
                sel,
                cfg,
                probes.as_ref(),
            ) {
                Ok((rec, skipped)) => {
                    for s in skipped {
                        out.emit(
                            &none,
                            &Diagnostic::warning(
                                "W0802",
                                format!("`{}` is not recorded in the waveform", s.path),
                            )
                            .with_note(format!(
                                "it has {} elements, more than `--wave-max-array={}`",
                                s.elems, a.wave_max_array
                            )),
                        );
                    }
                    sim.subscribe(Box::new(rec.clone()));
                    // Ctrl-C ends the run between cycles so the file is finished (rule 11).
                    let (stop, flag) = (sim.interrupt(), interrupted.clone());
                    if let Err(err) = yavs_wave::interrupt::install(move || {
                        flag.store(true, Ordering::Relaxed);
                        stop.stop();
                    }) {
                        out.emit(
                            &none,
                            &Diagnostic::error("E0803", "cannot install the Ctrl-C handler")
                                .with_note(err.to_string()),
                        );
                        return out.finish(2);
                    }
                    Some((path, rec))
                }
                Err(err) => {
                    out.emit(&none, &wave_error(path, err));
                    return out.finish(2);
                }
            }
        }
    };
    let r = sim.run_until(stop);
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
        out.emit(&none, &wave_error(path, err));
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
        let (cov, warns) = yavs_cov::collect(&db, &e, &counts, a.cov_exclude_translate_off);
        for w in &warns {
            out.emit(&none, w);
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
    let code = if out.errors > 0 { code.max(1) } else { code };
    out.finish(code)
}

/// Prints reports and `OUTPUT` text in NVC's format on stdout (cli-contract section 5;
/// tools/difftest parses it).
struct Printer {
    names: Vec<String>,
}

impl yavs_kernel::Observer for Printer {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        let sev = match r.severity {
            yavs_kernel::Severity::Note => "Note",
            yavs_kernel::Severity::Warning => "Warning",
            yavs_kernel::Severity::Error => "Error",
            yavs_kernel::Severity::Failure => "Failure",
        };
        let name = self
            .names
            .get(r.process.0 as usize)
            .map_or("?", |s| s.as_str());
        let mut o = std::io::stdout().lock();
        let _ = writeln!(
            o,
            "** {sev}: {}fs+{}: {}\n    Process {name}",
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

/// `10ms`, `1.5 ns`: a TIME literal.
fn parse_time(t: &str) -> Result<yavs_kernel::Time, String> {
    let fs = yavs_std::value::time_literal(t).map_err(|e| e.to_string())?;
    yavs_kernel::Time::from_vhdl(fs).map_err(|e| e.to_string())
}
