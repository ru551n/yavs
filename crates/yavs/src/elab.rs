//! `yavs elab` and `yavs run` (docs/cli-contract.md section 5).
//!
//! The design is re-analyzed from source: the closure of TOP is read from the library
//! manifests (each under a shared lock held only for the read, rule 14), every file is
//! analyzed with its recorded standard against one `std`/`ieee` revision (the newest in the
//! closure, or `--std`), then elaborated.

use std::process::ExitCode;

use clap::Args as ClapArgs;
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};
use yavs_lib::{Libraries, UnitKind, UnitRef};
use yavs_sem::{Db, LangRev, Options};

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use yavs_kernel::{EndReason, RunStatus};

use crate::StdArg;
use crate::analyze::{Out, stdlib};

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
    /// `-Werror` makes warnings errors.
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
    /// Stop the simulation at this time, e.g. `10ms`.
    #[arg(long, value_name = "TIME")]
    stop_time: Option<String>,
    /// Worker threads for process evaluation.
    #[arg(long, default_value = "1")]
    threads: usize,
    /// Randomize process execution order, with an optional seed.
    #[arg(long, value_name = "SEED", num_args = 0..=1, require_equals = true, default_missing_value = "1")]
    shuffle: Option<u64>,
    /// Load a shared library for foreign subprograms.
    #[arg(long, value_name = "LIB")]
    load: Vec<String>,
    /// Load a VHPI plugin, `LIB[:ENTRY]`.
    #[arg(long, value_name = "LIB[:ENTRY]")]
    vhpi: Vec<String>,
    /// Serve the Debug Adapter Protocol on this port.
    #[arg(long, value_name = "PORT")]
    dap: Option<u16>,
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
    format: MessageFormat,
    color: bool,
) -> Result<(Out, Db, yavs_elab::Elaborated), (Out, u8)> {
    let mut out = Out::new(format, color, c.warn.iter().any(|w| w == "error"));
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
    if let Some(w) = c.warn.iter().find(|w| *w != "error") {
        out.emit(
            &none,
            &Diagnostic::error("E0103", format!("unsupported warning option `-W{w}`")),
        );
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
    let mut opts = Options::new(rev, stdlib());
    opts.libraries = libs;
    opts.relaxed = c.relaxed;
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
            if let Err(ds) = e.lower(&mut db) {
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

pub fn elab(a: Common, format: MessageFormat, color: bool) -> ExitCode {
    match elaborate(&a, format, color) {
        Ok((out, db, e)) => {
            if a.print_hierarchy {
                print_hierarchy(&db, &e);
            }
            out.finish(0)
        }
        Err((out, code)) => out.finish(code),
    }
}

pub fn run(a: RunArgs, format: MessageFormat, color: bool) -> ExitCode {
    let (mut out, db, e) = match elaborate(&a.common, format, color) {
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
    for (given, what) in [
        (a.wave.is_some(), "`--wave` (M7)"),
        (!a.load.is_empty(), "`--load` (M3)"),
        (!a.vhpi.is_empty(), "`--vhpi` (M5)"),
        (a.dap.is_some(), "`--dap` (M8)"),
    ] {
        if given {
            out.emit(&none, &unsupported(what));
            return out.finish(2);
        }
    }
    let cfg = yavs_kernel::Config {
        threads: a.threads.max(1),
        shuffle: a.shuffle,
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
    let mut sim = match e.build(cfg) {
        Ok(s) => s,
        Err(err) => {
            out.emit(
                &none,
                &Diagnostic::error("E9999", format!("cannot build the simulation: {err}"))
                    .with_note("this is a limitation or a bug of yavs, not an error in the design"),
            );
            return out.finish(3);
        }
    };
    let names: Vec<String> = e
        .procs
        .iter()
        .map(|p| {
            let n = e.hier.path_name(p.scope, yavs_elab::Case::Lrm);
            n.strip_suffix(':').unwrap_or(&n).to_string()
        })
        .collect();
    let errors = Arc::new(AtomicBool::new(false));
    sim.subscribe(Box::new(Printer {
        names,
        errors: errors.clone(),
    }));
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
        Ok(_) => 0,
        Err(err) => {
            let msg = match &err.kind {
                yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
                k => k.to_string(),
            };
            let _ = writeln!(o, "** Fatal: {}fs+{}: {msg}", err.time.as_fs(), err.delta);
            if err.exit_status() == 101 { 3 } else { 1 }
        }
    };
    let _ = o.flush();
    drop(o);
    // A report of severity error or failure fails the run even below `--exit-severity`
    // (NVC does the same; VUnit only looks at zero or non-zero).
    let code = if code == 0 && errors.load(Ordering::Relaxed) {
        1
    } else {
        code
    };
    out.finish(code)
}

/// Prints reports and `OUTPUT` text in NVC's format on stdout (cli-contract section 5;
/// tools/difftest parses it).
struct Printer {
    names: Vec<String>,
    errors: Arc<AtomicBool>,
}

impl yavs_kernel::Observer for Printer {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        let sev = match r.severity {
            yavs_kernel::Severity::Note => "Note",
            yavs_kernel::Severity::Warning => "Warning",
            yavs_kernel::Severity::Error => "Error",
            yavs_kernel::Severity::Failure => "Failure",
        };
        if matches!(
            r.severity,
            yavs_kernel::Severity::Error | yavs_kernel::Severity::Failure
        ) {
            self.errors.store(true, Ordering::Relaxed);
        }
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

/// `10ms`, `1.5 ns`: a TIME literal (the `time'value` parser).
fn parse_time(t: &str) -> Result<yavs_kernel::Time, String> {
    let b = t.trim().as_bytes();
    let split = b
        .iter()
        .position(|c| c.is_ascii_alphabetic())
        .unwrap_or(b.len());
    let text = format!("{} {}", t.trim()[..split].trim(), &t.trim()[split..]);
    let fs = yavs_std::value::physical_value(
        text.as_bytes(),
        &yavs_std::types::time_type(),
        yavs_std::types::Bounds::I64,
    )
    .map_err(|e| e.to_string())?;
    yavs_kernel::Time::from_vhdl(fs).map_err(|e| format!("{e:?}"))
}
