//! `yavs analyze` and response-file expansion (docs/cli-contract.md sections 0, 4).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args as ClapArgs;
use yavs_diag::{Diagnostic, MessageFormat, Severity, SourceMap};
use yavs_lib::{Libraries, Library};
use yavs_sem::{Db, LangRev, Options};
use yavs_syntax::Encoding;

use crate::StdArg;

#[cfg_attr(test, derive(Debug))]
#[derive(ClapArgs)]
pub struct Args {
    /// Language standard of this file.
    #[arg(long, value_enum, default_value = "2008")]
    std: StdArg,
    /// The library analyzed into, `NAME:PATH`.
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
    /// Define a conditional analysis identifier, `NAME=VALUE`.
    #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = crate::parse_define)]
    defines: Vec<(String, String)>,
    /// Warning control: `-Werror` (every warning is an error), `-Werror=CODE`, `-Wno-CODE` (do not
    /// print it), `-WCODE` (stays a warning under `-Werror`).
    #[arg(short = 'W', value_name = "FLAG")]
    warn: Vec<String>,
    /// Allow `std`/`ieee` as `--work`/`--map` (analyzes yavs's own lib/ into a library).
    #[arg(long)]
    bootstrap: bool,
    /// Neither read nor write the analysed-unit cache of the libraries.
    #[arg(long)]
    no_cache: bool,
    /// Directory of the std/ieee units shared by all libraries, or `off` (default: `$YAVS_STD_CACHE`,
    /// else `yavs/std` in the user's cache directory).
    #[arg(long, value_name = "DIR|off")]
    std_cache: Option<std::ffi::OsString>,
    /// The files, analyzed in this order; the first failing file stops the batch.
    #[arg(required = true, value_name = "FILE")]
    files: Vec<PathBuf>,
}

/// Replace every `@FILE` argument (after the program name) by the lines of FILE.
pub fn expand_response_files(
    args: impl Iterator<Item = String>,
) -> Result<Vec<String>, Box<Diagnostic>> {
    let mut out = Vec::new();
    for (i, a) in args.enumerate() {
        let Some(path) = a.strip_prefix('@').filter(|_| i > 0) else {
            out.push(a);
            continue;
        };
        let text = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| String::from_utf8(b).map_err(|_| "not valid UTF-8".to_string()))
            .map_err(|e| {
                Box::new(
                    Diagnostic::error("E0101", format!("cannot read response file `{path}`"))
                        .with_note(e),
                )
            })?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        if text.is_empty() {
            continue;
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        out.extend(
            text.split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()),
        );
    }
    Ok(out)
}

pub(crate) struct Out {
    format: MessageFormat,
    color: bool,
    pub(crate) errors: usize,
    warnings: usize,
    werror: bool,
    /// Per-code `-W` actions, the last option for a code wins.
    codes: std::collections::HashMap<String, Action>,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Allow,
    Keep,
    Deny,
}

impl Out {
    pub(crate) fn new(format: MessageFormat, color: bool) -> Out {
        Out {
            format,
            color,
            errors: 0,
            warnings: 0,
            werror: false,
            codes: Default::default(),
        }
    }

    /// Apply the `-W` options in order: `error`, `error=CODE`, `no-CODE`, `CODE`.
    pub(crate) fn set_warn(&mut self, opts: &[String]) -> Result<(), Box<Diagnostic>> {
        for o in opts {
            let (action, code) = match o.as_str() {
                "error" => {
                    self.werror = true;
                    continue;
                }
                _ => match (o.strip_prefix("error="), o.strip_prefix("no-")) {
                    (Some(c), _) => (Action::Deny, c),
                    (_, Some(c)) => (Action::Allow, c),
                    _ => (Action::Keep, o.as_str()),
                },
            };
            if !code.starts_with('W') || crate::explain::explain(code).is_none() {
                return Err(Box::new(
                    Diagnostic::error("E0103", format!("unsupported warning option `-W{o}`"))
                        .with_help(
                            "use `-Werror`, `-Werror=CODE`, `-Wno-CODE` or `-WCODE` with a warning code; \
                             `yavs --explain CODE` describes a code",
                        ),
                ));
            }
            self.codes.insert(code.to_string(), action);
        }
        Ok(())
    }

    pub(crate) fn emit(&mut self, sources: &SourceMap, d: &Diagnostic) {
        let action = self.codes.get(d.code).copied();
        if d.severity == Severity::Warning && action == Some(Action::Allow) {
            return;
        }
        match d.severity {
            Severity::Error => self.errors += 1,
            Severity::Warning if action == Some(Action::Deny) => self.errors += 1,
            Severity::Warning if self.werror && action != Some(Action::Keep) => self.errors += 1,
            Severity::Warning => self.warnings += 1,
            Severity::Note => {}
        }
        crate::stdout::flush();
        let _ = std::io::stderr()
            .lock()
            .write_all(yavs_diag::render(sources, d, self.format, self.color).as_bytes());
    }
    pub(crate) fn finish(&self, code: u8) -> ExitCode {
        if self.errors > 0 && self.format == MessageFormat::Human {
            let s = if self.errors == 1 { "" } else { "s" };
            let d = Diagnostic::error(
                "",
                format!("aborting due to {} previous error{s}", self.errors),
            );
            crate::stdout::flush();
            let _ = std::io::stderr().lock().write_all(
                yavs_diag::render(&SourceMap::default(), &d, self.format, self.color).as_bytes(),
            );
        }
        crate::stdout::flush();
        ExitCode::from(code)
    }
}

pub fn run(a: Args, format: MessageFormat, color: bool, encoding: Encoding) -> ExitCode {
    let mut out = Out::new(format, color);
    let none = SourceMap::default();
    // Usage errors: exit 2.
    let mut libs = Libraries::new();
    for spec in std::iter::once(&a.work).chain(&a.maps) {
        let name = spec
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let r = if (name == "std" || name == "ieee") && !a.bootstrap {
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
    }
    if let Err(d) = out.set_warn(&a.warn) {
        out.emit(&none, &d);
        return out.finish(2);
    }
    let (work, _) = a.work.split_once(':').expect("checked by add_spec");
    let work_lib: Library = libs.get(work).expect("just added").clone();

    let rev = match a.std {
        StdArg::V1993 => LangRev::Vhdl1993,
        StdArg::V2008 => LangRev::Vhdl2008,
        StdArg::V2019 => LangRev::Vhdl2019,
    };
    let std_name = rev.as_str();
    let mut opts = Options::new(rev, yavs_sem::default_stdlib());
    // Developer check of the analysed-unit cache: every file is analysed a second time without
    // it, and the two analyses must be the same (`docs/decisions.md` "Analysed-unit cache").
    let check = std::env::var_os("YAVS_UNIT_CHECK").is_some();
    opts.libraries = libs;
    opts.relaxed = a.relaxed;
    opts.psl = a.psl;
    opts.unit_audit |= check;
    opts.defines = a.defines;
    opts.encoding = encoding;
    if !a.no_cache {
        let shared = crate::cache::shared_std_dir(a.std_cache.as_deref()).and_then(|d| {
            d.as_deref()
                .map(crate::cache::prepare_shared_std)
                .transpose()?;
            Ok(d)
        });
        match shared {
            Ok(d) => {
                opts.units =
                    crate::cache::Units::store_for(&opts, &work_lib.dir, d.as_deref(), true)
            }
            Err(d) => {
                out.emit(&none, &d);
                return out.finish(2);
            }
        }
    }

    // One exclusive hold on the work library for the whole batch (contract section 3).
    let session = match work_lib.session() {
        Ok(s) => s,
        Err(d) => {
            out.emit(&none, &d);
            return out.finish(1);
        }
    };

    std::panic::set_hook(Box::new(|_| {}));
    let current = std::cell::Cell::new(0usize);
    let done: std::cell::RefCell<Vec<String>> = Default::default();
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The process exits right after the batch: freeing every AST and HIR node one by one is
        // pure cost (about a tenth of a cold analysis), so the database is never dropped.
        let mut shadow = check.then(|| {
            let mut o = opts.clone();
            o.units = None;
            std::mem::ManuallyDrop::new(Db::new(o))
        });
        // Warnings and notes are shown for the files of this call only; a unit read from the library
        // was reported when its file was analysed (#584). Errors are always shown.
        let paths: std::collections::HashSet<PathBuf> = a
            .files
            .iter()
            .filter_map(|f| std::path::absolute(f).ok())
            .collect();
        let named = |sources: &SourceMap, d: &Diagnostic| match &d.primary {
            Some(l) if d.severity != Severity::Error => {
                std::path::absolute(&sources.get(l.span.file).path)
                    .is_ok_and(|p| paths.contains(&p))
            }
            _ => true,
        };
        let bodies_opts = opts.clone();
        let mut db = std::mem::ManuallyDrop::new(Db::new(opts));
        let mut shown = 0;
        for (i, file) in a.files.iter().enumerate() {
            current.set(i);
            // Stamped before the read: an edit in between is never trusted.
            let stamp = yavs_lib::Stamp::of(file);
            let hash = std::fs::read(file).map(|b| yavs_lib::hash_content(&b));
            let before = out.errors;
            let h = hash.as_deref().ok();
            let r = if shadow.is_some() {
                db.analyze_file_hashed(work, file, rev, h)
            } else {
                db.analyze_file_lazy(work, file, rev, h)
            };
            if let Some(sh) = shadow.as_mut() {
                let r2 = sh.analyze_file(work, file, rev);
                if r.is_ok() != r2.is_ok() {
                    panic!(
                        "the analysed-unit cache changed the outcome of `{}`",
                        file.display()
                    );
                }
                if let Some(why) = db.differs_from(sh) {
                    panic!(
                        "the analysed-unit cache gave another analysis of `{}`: {why}",
                        file.display()
                    );
                }
            }
            for d in &db.diagnostics()[shown..] {
                if named(&db.sources, d) {
                    out.emit(&db.sources, d);
                }
            }
            shown = db.diagnostics().len();
            let records = match r {
                Ok(r) if out.errors == before => r,
                _ => return Some(i),
            };
            // The file was readable or analysis would have failed.
            let hash = hash.unwrap_or_default();
            if let Err(d) =
                session.record_deferred(std_name, Path::new(file), &hash, stamp, records)
            {
                out.emit(&none, &d);
                return Some(i);
            }
            done.borrow_mut().push(file.display().to_string());
        }
        // The std/ieee package bodies every elaboration loads, stored once (their warnings stay
        // theirs: this session's output is not changed).
        let missing = db.builtin_bodies_missing();
        if !missing.is_empty() {
            let mut scratch = std::mem::ManuallyDrop::new(Db::new(bodies_opts.clone()));
            for (lib, pkg) in missing {
                let _ = scratch.load_body(&lib, &pkg);
            }
        }
        None
    }));
    // The index of the library, written once for the batch (#747): after a failure too, so that the
    // files before the failing one stay recorded.
    let flushed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.flush()));
    match &flushed {
        Ok(Err(d)) => out.emit(&none, d),
        Err(_) => out.emit(
            &none,
            &Diagnostic::error("E9999", "internal compiler error: this is a bug in yavs")
                .with_note("panicked while writing the library index"),
        ),
        Ok(Ok(())) => {}
    }
    let failed = match run {
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            out.errors += 1;
            let d = Diagnostic::error("E9999", "internal compiler error: this is a bug in yavs")
                .with_note(format!("panicked: {msg}"))
                .with_note(format!(
                    "while analyzing `{}`",
                    a.files[current.get()].display()
                ));
            let _ = std::io::stderr()
                .lock()
                .write_all(yavs_diag::render(&none, &d, format, color).as_bytes());
            note_batch(&a.files, current.get(), &done.borrow(), format, color);
            return out.finish(3);
        }
        Ok(f) => f,
    };
    if let Some(i) = failed {
        note_batch(&a.files, i, &done.borrow(), format, color);
        return out.finish(1);
    }
    if !matches!(flushed, Ok(Ok(()))) {
        return out.finish(1);
    }
    // The process exits now: the lock goes with it, and freeing the session's copy of the manifest
    // (thousands of small strings) is pure cost, as for the database.
    std::mem::forget(session);
    out.finish(0)
}

/// For a batch (more than one file) say where it stopped and what was recorded. Plain lines in
/// the human format; a note diagnostic in the others, so that the stream stays what the format
/// promises (#839).
fn note_batch(
    files: &[PathBuf],
    failed: usize,
    done: &[String],
    format: MessageFormat,
    color: bool,
) {
    if files.len() < 2 {
        return;
    }
    let rest = files.len() - failed - 1;
    let head = format!(
        "analysis stopped at `{}` (file {} of {}){}",
        files[failed].display(),
        failed + 1,
        files.len(),
        if rest == 0 {
            String::new()
        } else {
            format!("; the remaining {rest} file(s) were not analyzed")
        }
    );
    let text = if format == MessageFormat::Human {
        let mut m = format!("{head}\n");
        if done.is_empty() {
            m.push_str("no file of this call was analyzed successfully\n");
        } else {
            m.push_str(&format!(
                "analyzed successfully and recorded before it ({}):\n",
                done.len()
            ));
            for f in done {
                m.push_str(&format!("  {f}\n"));
            }
        }
        m
    } else {
        let recorded = if done.is_empty() {
            "no file of this call was analyzed successfully".to_string()
        } else {
            format!(
                "analyzed successfully and recorded before it ({}): {}",
                done.len(),
                done.join(", ")
            )
        };
        // The short format has the message only; the JSON one has the details as a child.
        let d = if format == MessageFormat::Short {
            Diagnostic::new(Severity::Note, "", format!("{head}; {recorded}"))
        } else {
            Diagnostic::new(Severity::Note, "", head).with_note(recorded)
        };
        yavs_diag::render(&SourceMap::default(), &d, format, color)
    };
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}
