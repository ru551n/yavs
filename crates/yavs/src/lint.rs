//! `yavs lint`: reads the given files, or those recorded in the work library, with the analysis
//! `yavs analyze` would do, writes nothing into any library, and runs the rules of `yavs-lint`.
//! Exit status: 0 no finding of severity error, 1 such a finding, 2 usage or analysis error.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args as ClapArgs;
use yavs_diag::{Diagnostic, MessageFormat, Severity, SourceMap};
use yavs_lib::Libraries;
use yavs_lint::{Certainty, Config, Reports, Waivers};
use yavs_sem::{Db, LangRev, Options};
use yavs_syntax::Encoding;

use crate::StdArg;
use crate::analyze::Out;

/// `yavs-lint.yaml` looked for in the working directory and its ancestors.
const CONFIG: &str = "yavs-lint.yaml";
/// `yavs-lint-waivers.yaml` next to the configuration, else looked for like it.
const WAIVERS: &str = "yavs-lint-waivers.yaml";

#[cfg_attr(test, derive(Debug))]
#[derive(ClapArgs)]
pub struct Args {
    /// The files and directories to check (`.vhd`, `.vhdl`), read in this order. Default: every
    /// file recorded in the work library, which then needs `--work`.
    #[arg(value_name = "FILE|DIR")]
    inputs: Vec<PathBuf>,
    /// Look for files in the subdirectories of a directory too.
    #[arg(long, short = 'r')]
    recursive: bool,
    /// The library the files belong to, `NAME:PATH`. Without it they are read as library `work`.
    /// Nothing is written into it; a file recorded in a library is read in the order of the library.
    #[arg(long, value_name = "NAME:PATH")]
    work: Option<String>,
    /// Another library, `NAME:PATH` (repeatable).
    #[arg(long = "map", value_name = "NAME:PATH")]
    maps: Vec<String>,
    /// Language standard: the design revision, and the standard of a file no library records.
    /// Default: 2008 for such a file, and the newest standard among the files for the design.
    #[arg(long, value_enum)]
    std: Option<StdArg>,
    /// The configuration file. Default: `yavs-lint.yaml` in the working directory or an ancestor. Fails with E0134.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// The waiver file. Default: `yavs-lint-waivers.yaml` next to the configuration, or in the working directory or an ancestor. Fails with E0134.
    #[arg(long, value_name = "FILE")]
    waivers: Option<PathBuf>,
    /// Print every rule with its class and whether this configuration runs it, and exit.
    #[arg(long)]
    list_rules: bool,
    /// Print what a rule reports and what it needs to report it, and exit.
    #[arg(long, value_name = "RULE")]
    explain: Option<String>,
    /// Write the findings as JSON. Fails with E0135.
    #[arg(long, value_name = "FILE")]
    json: Option<PathBuf>,
    /// Write the findings as SARIF 2.1.0 for code scanning. Fails with E0135.
    #[arg(long, value_name = "FILE")]
    sarif: Option<PathBuf>,
    /// Write the findings as a JUnit file, one test case per source file. Fails with E0135.
    #[arg(long, value_name = "FILE")]
    junit: Option<PathBuf>,
    /// Write the findings as a Code Climate report (GitLab code quality). Fails with E0135.
    #[arg(long, value_name = "FILE")]
    quality_report: Option<PathBuf>,
    /// Write the findings as SonarQube generic issues. Fails with E0135.
    #[arg(long, value_name = "FILE")]
    sonarqube: Option<PathBuf>,
    /// Finding control, by rule id: `-Werror` (a finding of severity warning is an error),
    /// `-Werror=RULE`, `-Wno-RULE` (do not report it), `-WRULE` (stays a warning under `-Werror`).
    #[arg(short = 'W', value_name = "FLAG")]
    warn: Vec<String>,
    /// Turn some analysis errors into warnings, as `yavs analyze --relaxed` does.
    #[arg(long)]
    relaxed: bool,
    /// Read `-- psl` comments as PSL pragmas (native PSL statements are always read).
    #[arg(long)]
    psl: bool,
    /// Define a conditional analysis identifier, `NAME=VALUE`: the same definitions as when the
    /// files were analyzed, because the sources are read again.
    #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = crate::parse_define)]
    defines: Vec<(String, String)>,
    /// Do not read the analysed-unit cache of the libraries.
    #[arg(long)]
    no_cache: bool,
    /// Directory of the std/ieee units shared by all libraries, or `off` (default: `$YAVS_STD_CACHE`,
    /// else `yavs/std` in the user's cache directory).
    #[arg(long, value_name = "DIR|off")]
    std_cache: Option<std::ffi::OsString>,
}

/// What is known about one file to check.
struct Source {
    library: String,
    path: PathBuf,
    rev: Option<LangRev>,
}

fn parse_rev(s: &str) -> Option<LangRev> {
    match s {
        "1993" => Some(LangRev::Vhdl1993),
        "2008" => Some(LangRev::Vhdl2008),
        "2019" => Some(LangRev::Vhdl2019),
        _ => None,
    }
}

/// The `.vhd` and `.vhdl` files of a directory, sorted.
fn vhdl_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                vhdl_files(&p, true, out)?;
            }
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("vhd") || e.eq_ignore_ascii_case("vhdl"))
        {
            out.push(p);
        }
    }
    Ok(())
}

fn input_error(message: String) -> Box<Diagnostic> {
    Box::new(Diagnostic::error("E0133", message))
}

/// The files to check: those recorded in a library in the order of the library, then the others
/// in the order given.
fn sources(a: &Args, libs: &Libraries, work: &str) -> yavs_lib::Result<Vec<Source>> {
    let mut recorded: Vec<Source> = Vec::new();
    for name in std::iter::once(work).chain(libs.names()) {
        let Some(l) = libs.get(name).filter(|l| l.has_manifest()) else {
            continue;
        };
        if recorded.iter().any(|s| s.library == l.name) {
            continue;
        }
        for f in l.read()?.files() {
            recorded.push(Source {
                library: l.name.clone(),
                path: f.path.to_path_buf(),
                rev: parse_rev(f.std),
            });
        }
    }
    if a.inputs.is_empty() {
        recorded.retain(|s| s.library == yavs_lib::fold(work));
        if recorded.is_empty() {
            return Err(input_error(
                "nothing to check: no file was given and the work library records none".into(),
            )
            .with_help("name files or directories, or give `--work` with an analyzed library")
            .into());
        }
        return Ok(recorded);
    }
    let mut files = Vec::new();
    for i in &a.inputs {
        if i.is_dir() {
            vhdl_files(i, a.recursive, &mut files).map_err(|e| {
                input_error(format!("cannot read directory `{}`: {e}", i.display()))
            })?;
        } else if i.is_file() {
            files.push(i.clone());
        } else {
            return Err(input_error(format!(
                "no such file or directory: `{}`",
                i.display()
            )));
        }
    }
    let abs: Vec<PathBuf> = files
        .iter()
        .map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone()))
        .collect();
    let mut out: Vec<Source> = recorded
        .into_iter()
        .filter(|s| abs.contains(&s.path))
        .collect();
    let known: Vec<PathBuf> = out.iter().map(|s| s.path.clone()).collect();
    for (f, abs) in files.into_iter().zip(&abs) {
        if !known.contains(abs) && !out.iter().any(|s| s.path == f) {
            out.push(Source {
                library: yavs_lib::fold(work),
                path: f,
                rev: None,
            });
        }
    }
    if out.is_empty() {
        return Err(input_error("no VHDL file in the given directories".into())
            .with_help("files end in `.vhd` or `.vhdl`; `--recursive` looks in subdirectories too")
            .into());
    }
    Ok(out)
}

fn read_file(path: &Path) -> Result<String, Box<Diagnostic>> {
    std::fs::read_to_string(path).map_err(|e| {
        Box::new(Diagnostic::error(
            "E0134",
            format!("cannot read `{}`: {e}", path.display()),
        ))
    })
}

/// The configuration and waivers named or found, with their errors naming the file.
fn load_settings(a: &Args) -> Result<(Config, Waivers), Box<Diagnostic>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let bad = |p: &Path, e: String| {
        Box::new(Diagnostic::error("E0134", format!("invalid file `{}`", p.display())).with_note(e))
    };
    let config = a
        .config
        .clone()
        .or_else(|| yavs_lint::find_up(&cwd, CONFIG));
    let cfg = match &config {
        Some(p) => Config::parse(&read_file(p)?).map_err(|e| bad(p, e))?,
        None => Config::default(),
    };
    let waivers = a.waivers.clone().or_else(|| {
        config
            .as_deref()
            .and_then(Path::parent)
            .map(|d| d.join(WAIVERS))
            .filter(|p| p.is_file())
            .or_else(|| yavs_lint::find_up(&cwd, WAIVERS))
    });
    let w = match &waivers {
        Some(p) => Waivers::parse(&read_file(p)?).map_err(|e| bad(p, e))?,
        None => Waivers::default(),
    };
    Ok((cfg, w))
}

fn class(c: Certainty) -> &'static str {
    match c {
        Certainty::Definite => "definite error",
        c => c.name(),
    }
}

/// `--explain` and `yavs --explain`: the text of a rule.
pub(crate) fn explain_text(id: &str) -> Option<String> {
    yavs_lint::rule(id).map(|r| {
        format!(
            "{} ({}): {}\n{}",
            r.id,
            class(r.certainty),
            r.summary,
            r.explain
        )
    })
}

pub fn run(a: Args, format: MessageFormat, color: bool, encoding: Encoding) -> ExitCode {
    let mut out = Out::new(format, color);
    let none = SourceMap::default();
    if let Some(id) = &a.explain {
        return match explain_text(id) {
            Some(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            None => {
                out.emit(
                    &none,
                    &Diagnostic::error("E0133", format!("no such rule `{id}`"))
                        .with_help("`yavs lint --list-rules` lists the rules"),
                );
                out.finish(2)
            }
        };
    }
    let (cfg, waivers) = match load_settings(&a) {
        Ok(s) => s,
        Err(d) => {
            out.emit(&none, &d);
            return out.finish(2);
        }
    };
    if a.list_rules {
        for r in yavs_lint::RULES {
            let on = if cfg.enabled(r) { "on" } else { "off" };
            println!("{} {:<14} {on:<3} {}", r.id, class(r.certainty), r.summary);
        }
        return ExitCode::SUCCESS;
    }
    let lint_code = |c: &str| yavs_lint::rule(c).is_some();
    if let Err(d) = out.set_warn_with(&a.warn, &lint_code) {
        out.emit(&none, &d);
        return out.finish(2);
    }
    let mut libs = Libraries::new();
    let mut work = "work".to_string();
    for (i, spec) in a.work.iter().chain(&a.maps).enumerate() {
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
            return out.finish(2);
        }
        if i == 0 && a.work.is_some() {
            work = name;
        }
    }
    let sources = match sources(&a, &libs, &work) {
        Ok(s) => s,
        Err(d) => {
            out.emit(&none, &d);
            return out.finish(2);
        }
    };
    let std_rev = a.std.map(|s| match s {
        StdArg::V1993 => LangRev::Vhdl1993,
        StdArg::V2008 => LangRev::Vhdl2008,
        StdArg::V2019 => LangRev::Vhdl2019,
    });
    let file_rev = |s: &Source| s.rev.or(std_rev).unwrap_or(LangRev::Vhdl2008);
    let rev = std_rev
        .or_else(|| {
            sources
                .iter()
                .map(file_rev)
                .max_by_key(|r| r.as_str().to_string())
        })
        .unwrap_or(LangRev::Vhdl2008);
    let mut opts = Options::new(rev, yavs_sem::default_stdlib());
    opts.libraries = libs;
    opts.relaxed = a.relaxed;
    opts.psl = a.psl;
    opts.defines = a.defines.clone();
    opts.encoding = encoding;
    if !a.no_cache {
        let shared = match crate::cache::shared_std_dir(a.std_cache.as_deref()) {
            Ok(d) => d,
            Err(d) => {
                out.emit(&none, &d);
                return out.finish(2);
            }
        };
        let work_dir = opts.libraries.get(&work).map(|l| l.dir.clone());
        // Read-only: a library is the business of `yavs analyze`.
        if let Some(dir) = work_dir.as_deref().or(shared.as_deref()) {
            opts.units = crate::cache::Units::store_for(&opts, dir, shared.as_deref(), false);
        }
    }

    // The process exits right after: freeing every AST and HIR node is pure cost, as in `analyze`.
    let mut db = std::mem::ManuallyDrop::new(Db::new(opts));
    let mut failed = false;
    let mut shown = 0;
    let mut checked: Vec<String> = Vec::new();
    for s in &sources {
        let r = db.analyze_file(&s.library, &s.path, file_rev(s));
        // `yavs analyze` prints the warnings of analysis; this run adds only what stops it.
        for d in &db.diagnostics()[shown..] {
            if d.severity == Severity::Error {
                out.emit(&db.sources, d);
            }
        }
        shown = db.diagnostics().len();
        if r.is_ok() {
            checked.push(s.path.display().to_string());
        } else {
            failed = true;
        }
    }
    let units: Vec<_> = db
        .units
        .iter()
        .filter(|(_, u)| checked.iter().any(|p| *p == db.sources.get(u.file).path))
        .map(|(id, _)| id)
        .collect();

    let mut entries = Vec::new();
    let mut waived = 0usize;
    let analysis_errors = out.errors;
    for f in yavs_lint::lint(&db, &units, &cfg) {
        let file = db.sources.get(f.span.file);
        let (line, column) = file.line_col(f.span.start);
        if waivers.waives(&file.path, f.rule.id, line) {
            waived += 1;
            continue;
        }
        if !out.keeps(f.rule.id) {
            continue;
        }
        let d = Diagnostic::new(f.severity, f.rule.id, f.message.clone())
            .with_primary(f.span, "")
            .with_help(format!(
                "`yavs lint --explain {}` says what this rule reports",
                f.rule.id
            ));
        let before = out.errors;
        out.emit(&db.sources, &d);
        entries.push(yavs_lint::Entry {
            file: file.path.clone(),
            rule: f.rule,
            line,
            column,
            error: out.errors > before,
            message: f.message,
        });
    }
    let reports = Reports {
        files: &checked,
        entries: &entries,
    };
    let writes = [
        (&a.json, &(|| reports.json()) as &dyn Fn() -> String),
        (&a.sarif, &|| reports.sarif()),
        (&a.junit, &|| reports.junit()),
        (&a.quality_report, &|| reports.quality()),
        (&a.sonarqube, &|| reports.sonarqube()),
    ];
    for (path, text) in writes {
        if let Some(p) = path
            && let Err(e) = std::fs::write(p, text())
        {
            out.emit(
                &none,
                &Diagnostic::error("E0135", format!("cannot write `{}`: {e}", p.display())),
            );
            failed = true;
        }
    }
    let findings = entries.len();
    if format == MessageFormat::Human {
        let s = if findings == 1 { "" } else { "s" };
        let more = if waived > 0 {
            format!(" ({waived} waived)")
        } else {
            String::new()
        };
        let d = Diagnostic::new(
            Severity::Note,
            "",
            format!("{findings} finding{s} in {} file(s){more}", checked.len()),
        );
        out.emit(&none, &d);
    }
    if failed {
        ExitCode::from(2)
    } else if out.errors > analysis_errors {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
