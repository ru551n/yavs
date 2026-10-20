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
    /// Relax some LRM checks (sem.md 8.4).
    #[arg(long)]
    relaxed: bool,
    /// Read `-- psl` comments as PSL pragmas (native PSL statements are always read).
    #[arg(long)]
    psl: bool,
    /// Define a conditional analysis identifier, `NAME=VALUE`.
    #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = crate::parse_define)]
    defines: Vec<(String, String)>,
    /// Warning control: `-Werror`, `-Werror=CODE`, `-Wno-CODE`, `-WCODE` (docs/cli-contract.md 4).
    #[arg(short = 'W', value_name = "FLAG")]
    warn: Vec<String>,
    /// Allow `std`/`ieee` as `--work`/`--map` (analyzes yavs's own lib/ into a library).
    #[arg(long)]
    bootstrap: bool,
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
            let _ = std::io::stderr().lock().write_all(
                yavs_diag::render(&SourceMap::default(), &d, self.format, self.color).as_bytes(),
            );
        }
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
        StdArg::V2008 => LangRev::Vhdl2008,
        StdArg::V2019 => LangRev::Vhdl2019,
    };
    let std_name = if matches!(a.std, StdArg::V2008) {
        "2008"
    } else {
        "2019"
    };
    let mut opts = Options::new(rev, yavs_sem::default_stdlib());
    opts.libraries = libs;
    opts.relaxed = a.relaxed;
    opts.psl = a.psl;
    opts.defines = a.defines;
    opts.encoding = encoding;

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
        let mut db = Db::new(opts);
        let mut shown = 0;
        for (i, file) in a.files.iter().enumerate() {
            current.set(i);
            // Stamped before the read: an edit in between is never trusted.
            let stamp = yavs_lib::Stamp::of(file);
            let bytes = std::fs::read(file);
            let before = out.errors;
            let r = db.analyze_file(work, file, rev);
            for d in &db.diagnostics()[shown..] {
                out.emit(&db.sources, d);
            }
            shown = db.diagnostics().len();
            let records = match r {
                Ok(r) if out.errors == before => r,
                _ => return Some(i),
            };
            // bytes was readable or analysis would have failed.
            let hash = yavs_lib::hash_content(&bytes.unwrap_or_default());
            if let Err(d) = session.record_stamped(std_name, Path::new(file), &hash, stamp, records)
            {
                out.emit(&none, &d);
                return Some(i);
            }
            done.borrow_mut().push(file.display().to_string());
        }
        None
    }));
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
            note_batch(&a.files, current.get(), &done.borrow());
            return out.finish(3);
        }
        Ok(f) => f,
    };
    if let Some(i) = failed {
        note_batch(&a.files, i, &done.borrow());
        return out.finish(1);
    }
    out.finish(0)
}

/// For a batch (more than one file) say where it stopped and what was recorded.
fn note_batch(files: &[PathBuf], failed: usize, done: &[String]) {
    if files.len() < 2 {
        return;
    }
    let mut m = format!(
        "analysis stopped at `{}` (file {} of {}); the remaining {} file(s) were not analyzed\n",
        files[failed].display(),
        failed + 1,
        files.len(),
        files.len() - failed - 1
    );
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
    let _ = std::io::stderr().lock().write_all(m.as_bytes());
}
