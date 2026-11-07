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
    /// The files and directories to check (`.vhd`, `.vhdl`), read in this order, before those of
    /// `file_list` in the configuration. With neither: every file recorded in the work library,
    /// which then needs `--work`.
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
    /// The configuration file. Default: `yavs-lint.yaml` in the working directory or an ancestor. A bad value, an unknown key or an unknown rule fails with E0134, all of them listed.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// The waiver file. Default: `yavs-lint-waivers.yaml` next to the configuration, or in the working directory or an ancestor. A bad value, an unknown key or an unknown rule fails with E0134, all of them listed.
    #[arg(long, value_name = "FILE")]
    waivers: Option<PathBuf>,
    /// Do not check the files of `file_list` in the configuration.
    #[arg(long)]
    no_file_list: bool,
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
    /// Write the design facts as JSON: state machines, clock domains, reset domains and crossings
    /// of each architecture. Fails with E0135.
    #[arg(long, value_name = "FILE")]
    facts: Option<PathBuf>,
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
    /// The standard a library records for it; `None` for a file no library records.
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

/// The configuration file named or found.
fn config_path(a: &Args) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    a.config
        .clone()
        .or_else(|| yavs_lint::find_up(&cwd, CONFIG))
}

/// The files `file_list` names, in its order. An entry that gives no file is an error.
fn listed(a: &Args, cfg: &Config) -> Result<Vec<PathBuf>, Box<Diagnostic>> {
    let config = config_path(a).unwrap_or_default();
    let error = |text: String| {
        let at = format!("`file_list` in `{}`", config.display());
        Box::new(Diagnostic::error("E0134", format!("{at}: {text}")))
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut out = Vec::new();
    for entry in cfg.file_list() {
        let mut files = Vec::new();
        let found = yavs_lint::expand(entry).map_err(error)?;
        for f in &found {
            if f.is_dir() {
                vhdl_files(f, a.recursive, &mut files)
                    .map_err(|e| error(format!("cannot read directory `{}`: {e}", f.display())))?;
            } else {
                files.push(f.clone());
            }
        }
        if files.is_empty() {
            let what = if found.is_empty() {
                "matches no file"
            } else {
                "is a directory with no VHDL file (`--recursive` also looks in its subdirectories)"
            };
            return Err(error(format!(
                "the entry `{entry}` {what}: it is relative to the working directory `{}`, so correct the entry or run from the directory it is relative to{}",
                cwd.display(),
                yavs_lint::near_entry(entry),
            )));
        }
        out.append(&mut files);
    }
    Ok(out)
}

/// The files to check: those recorded in a library in the order of the library, then the others
/// in the order given, those of `file_list` last. A file named twice is read once.
fn sources(a: &Args, cfg: &Config, libs: &Libraries, work: &str) -> yavs_lib::Result<Vec<Source>> {
    let mut recorded: Vec<Source> = Vec::new();
    for name in std::iter::once(work).chain(libs.names()) {
        let Some(l) = libs.get(name).filter(|l| l.has_manifest()) else {
            continue;
        };
        if recorded.iter().any(|s| s.library == l.name) {
            continue;
        }
        // A recorded Verilog file is not VHDL to lint.
        for f in l.read()?.files().filter(|f| f.std != yavs_lib::VERILOG_STD) {
            recorded.push(Source {
                library: l.name.clone(),
                path: f.path.to_path_buf(),
                rev: parse_rev(f.std),
            });
        }
    }
    let list = if a.no_file_list {
        Vec::new()
    } else {
        listed(a, cfg)?
    };
    if a.inputs.is_empty() && list.is_empty() {
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
    files.extend(list);
    // A file is one file however it is spelled: a recorded one keeps the library's standard.
    let key = |p: &Path| yavs_diag::files::resolve(p);
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(key(f)));
    let mut out: Vec<Source> = recorded
        .into_iter()
        .filter(|s| seen.contains(&key(&s.path)))
        .collect();
    let known: Vec<PathBuf> = out.iter().map(|s| key(&s.path)).collect();
    for f in files {
        if !known.contains(&key(&f)) && !out.iter().any(|s| s.path == f) {
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

/// The configuration, the waivers, the directories their patterns are relative to.
type Settings = (Config, Waivers, PathBuf, PathBuf);

/// The configuration and waivers named or found, with their errors naming the file.
fn load_settings(a: &Args) -> Result<Settings, Box<Diagnostic>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    // In the message: the short format, which CI reads, prints no notes. Several errors of one
    // file are one line each below the file's name.
    let bad = |p: &Path, e: String| {
        let shown = p.strip_prefix(&cwd).unwrap_or(p);
        let at = format!("invalid file `{}`:", shown.display());
        let text = if e.contains('\n') {
            format!("{at}\n  {}", e.replace('\n', "\n  "))
        } else {
            format!("{at} {e}")
        };
        Box::new(Diagnostic::error("E0134", text))
    };
    let config = config_path(a);
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
    // Relative to the configuration file, and only ever used with its patterns.
    let base = config
        .as_deref()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or(cwd);
    // A waiver's patterns are relative to the waiver file.
    let waiver_base = waivers
        .as_deref()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| base.clone());
    Ok((cfg, w, base, waiver_base))
}

/// W0132: a waiver pattern that covers no file of this run and no VHDL file under the waiver file.
/// A warning: a shared waiver file may name generated files that are not there yet.
fn stale_patterns(w: &Waivers, checked: &[String], base: &Path, out: &mut Out, none: &SourceMap) {
    let left = yavs_lint::unmatched(w.patterns(), checked, base);
    if left.is_empty() {
        return;
    }
    let mut on_disk = Vec::new();
    if let Err(e) = vhdl_files(base, true, &mut on_disk) {
        let message = format!("cannot look for what the waiver patterns cover: {e}");
        out.emit(none, &Diagnostic::new(Severity::Warning, "W0132", message));
        return;
    }
    let on_disk: Vec<String> = on_disk.iter().map(|p| p.display().to_string()).collect();
    for (line, pattern) in yavs_lint::unmatched(left, &on_disk, base) {
        let message = format!(
            "the waiver pattern `{pattern}` (line {line} of the waiver file) covers no file: it is relative to `{}`",
            base.display()
        );
        out.emit(none, &Diagnostic::new(Severity::Warning, "W0132", message));
    }
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
                let mut d = Diagnostic::error("E0133", format!("no such rule `{id}`"));
                if let Some(h) = crate::explain::suggest_code(id, crate::explain::Codes::None, true)
                {
                    d = d.with_help(h);
                }
                out.emit(
                    &none,
                    &d.with_help("`yavs lint --list-rules` lists the rules"),
                );
                out.finish(2)
            }
        };
    }
    let (cfg, waivers, base, waiver_base) = match load_settings(&a) {
        Ok(s) => s,
        Err(mut d) => {
            // The short format is one line per diagnostic; only the human one lists several errors.
            if !matches!(format, MessageFormat::Human) {
                d.message = d.message.replace(":\n  ", ": ").replace("\n  ", "; ");
            }
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
    if let Err(d) = out.set_warn_with(&a.warn, &lint_code, true) {
        out.emit(&none, &d);
        return out.finish(2);
    }
    let settings_errors = out.errors;
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
    let sources = match sources(&a, &cfg, &libs, &work) {
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
        // What `exclude` covers is not checked. A file a library records is found there if another
        // file needs it; a loose one is read, as a later file may need it.
        let excluded = cfg.excluded(
            &std::path::absolute(&s.path).unwrap_or_else(|_| s.path.clone()),
            &base,
        );
        if excluded && s.rev.is_some() {
            continue;
        }
        let r = db.analyze_file(&s.library, &s.path, file_rev(s));
        // `yavs analyze` prints the warnings of analysis; this run adds only what stops it.
        for d in &db.diagnostics()[shown..] {
            if d.severity == Severity::Error {
                out.emit(&db.sources, d);
            }
        }
        shown = db.diagnostics().len();
        match r {
            Ok(_) if !excluded => checked.push(s.path.display().to_string()),
            Ok(_) => {}
            Err(_) => failed = true,
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
    let (found, facts) = yavs_lint::lint(&db, &units, &cfg, &base, a.facts.is_some());
    for f in found {
        let file = db.sources.get(f.span.file);
        let (line, column) = file.line_col(f.span.start);
        if waivers.waives(&file.path, &waiver_base, f.rule.id, line) {
            waived += 1;
            continue;
        }
        if !out.keeps(f.rule.id) {
            continue;
        }
        let mut d = Diagnostic::new(f.severity, f.rule.id, f.message.clone())
            .with_primary(f.span, "")
            .with_help(format!(
                "`yavs lint --explain {}` says what this rule reports",
                f.rule.id
            ));
        for &other in f.related.iter().filter(|o| **o != f.span) {
            d = d.with_secondary(other, "also here");
        }
        let before = out.errors;
        out.emit(&db.sources, &d);
        entries.push(yavs_lint::Entry {
            file: file.path.clone(),
            rule: f.rule,
            line,
            column,
            error: out.errors > before,
            related: f
                .related
                .iter()
                .filter(|o| **o != f.span)
                .map(|o| {
                    let file = db.sources.get(o.file);
                    let (line, column) = file.line_col(o.start);
                    (file.path.clone(), line, column)
                })
                .collect(),
            message: f.message,
        });
    }
    stale_patterns(&waivers, &checked, &waiver_base, &mut out, &none);
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
        (&a.facts, &|| {
            serde_json::to_string_pretty(&facts).expect("a JSON value") + "\n"
        }),
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
    } else if settings_errors > 0 || out.errors > analysis_errors {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
