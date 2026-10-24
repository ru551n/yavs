//! `yavs cov merge`, `yavs cov report` and `yavs cov rank` (docs/cli-contract.md section 5).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use yavs_cov::Database;
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};

use crate::analyze::Out;

/// The form of `--uncovered`.
#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum UncoveredFormat {
    Text,
    Yaml,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub enum Command {
    /// Add up the counts of several coverage databases into one.
    Merge {
        /// The merged database.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
        #[arg(required = true, value_name = "DB")]
        inputs: Vec<PathBuf>,
    },
    /// Print a text summary of coverage databases (merged), and write other reports.
    Report {
        /// Also write a Cobertura XML report.
        #[arg(long, value_name = "FILE.xml")]
        cobertura: Option<PathBuf>,
        /// Also write an HTML report (static files, sources read from the paths in the
        /// database) into DIR.
        #[arg(long, value_name = "DIR")]
        html: Option<PathBuf>,
        /// Strip DIR from the front of the source paths the HTML report shows (default: the
        /// current directory). Sources are still read from the paths in the database.
        #[arg(long, value_name = "DIR", requires = "html")]
        root: Option<PathBuf>,
        /// Leave out the items an exclusion file lists (rows pasted from `--uncovered`; a
        /// `.yaml` or `.yml` file is read as YAML).
        #[arg(long, value_name = "FILE")]
        exclude: Option<PathBuf>,
        /// Print only the items that never ran, as exclusion rows (`--uncovered=yaml`: as a
        /// YAML exclusion file).
        #[arg(
            long,
            value_name = "FORMAT",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = "text"
        )]
        uncovered: Option<UncoveredFormat>,
        /// Print every item that ran, as an exclusion row with the tests that hit it.
        #[arg(long, conflicts_with = "uncovered")]
        tests: bool,
        /// Warning control, as for `analyze`: `-Werror`, `-Wno-CODE`, ...
        #[arg(short = 'W', value_name = "FLAG")]
        warn: Vec<String>,
        #[arg(required = true, value_name = "DB")]
        inputs: Vec<PathBuf>,
    },
    /// Rank the tests of merged databases: a minimal set reaching the same coverage, and
    /// what each test alone covers.
    Rank {
        /// Leave out the items an exclusion file lists.
        #[arg(long, value_name = "FILE")]
        exclude: Option<PathBuf>,
        /// Warning control, as for `analyze`: `-Werror`, `-Wno-CODE`, ...
        #[arg(short = 'W', value_name = "FLAG")]
        warn: Vec<String>,
        #[arg(required = true, value_name = "DB")]
        inputs: Vec<PathBuf>,
    },
}

/// Cannot write a coverage database or report.
const WRITE_ERROR: &str = "E0110";
/// Cannot read or parse an exclusion file.
const EXCLUDE_ERROR: &str = "E0112";
/// Cannot read a coverage database.
const READ_ERROR: &str = "E0111";
/// Cannot read a source file named by the database (`--html`).
const SOURCE_ERROR: &str = "E0113";

fn merged(inputs: &[PathBuf]) -> Result<Database, Box<Diagnostic>> {
    let mut db = Database::default();
    for p in inputs {
        let d = std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| Database::parse(&t));
        match d {
            Ok(d) => db.merge(d),
            Err(m) => {
                return Err(Box::new(
                    Diagnostic::error(
                        READ_ERROR,
                        format!("cannot read coverage database `{}`", p.display()),
                    )
                    .with_note(m),
                ));
            }
        }
    }
    Ok(db)
}

fn write(path: &PathBuf, text: String) -> Result<(), Box<Diagnostic>> {
    std::fs::write(path, text).map_err(|e| {
        Box::new(
            Diagnostic::error(WRITE_ERROR, format!("cannot write `{}`", path.display()))
                .with_note(e.to_string()),
        )
    })
}

/// Excluded items with the reasons of their rows.
type Excluded = Vec<(yavs_cov::Key, yavs_cov::Entry, String)>;

/// `db` without the items the exclusion file lists, and those items.
fn excluded(
    out: &mut Out,
    db: Database,
    exclude: Option<PathBuf>,
) -> Result<(Database, Excluded), Box<Diagnostic>> {
    let Some(p) = &exclude else {
        return Ok((db, Vec::new()));
    };
    let fail = |m: String| {
        Box::new(
            Diagnostic::error(
                EXCLUDE_ERROR,
                format!("cannot use exclusion file `{}`", p.display()),
            )
            .with_note(m),
        )
    };
    let yaml = p
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"));
    let rows = std::fs::read_to_string(p)
        .map_err(|e| e.to_string())
        .and_then(|t| {
            if yaml {
                yavs_cov::exclude::parse_yaml(&t)
            } else {
                yavs_cov::exclude::parse(&t)
            }
        })
        .map_err(fail)?;
    let a = yavs_cov::exclude::apply(&db, &rows, &p.display().to_string());
    for w in &a.warnings {
        out.emit(&SourceMap::default(), w);
    }
    Ok((a.kept, a.excluded))
}

/// Split a path into (Windows style, components) with `.` and `..` collapsed lexically. A path
/// is Windows style when it starts with a drive letter or `\\` / `//`: then both separators
/// split, `\\?\` (verbatim) prefixes drop and the drive letter folds to lower case.
// ponytail: lexical, symlinks are not resolved; canonicalize both sides if that ever matters.
fn components(p: &str) -> (bool, Vec<String>) {
    let b = p.as_bytes();
    let win = (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || p.starts_with("\\\\")
        || p.starts_with("//");
    let p = if win {
        p.replace('\\', "/")
    } else {
        p.to_string()
    };
    let p = p
        .strip_prefix("//?/UNC/")
        .map(|r| format!("//{r}"))
        .or_else(|| p.strip_prefix("//?/").map(str::to_string))
        .unwrap_or(p);
    let mut out: Vec<String> = Vec::new();
    for (i, c) in p.split('/').enumerate() {
        match c {
            "." => {}
            ".." if out.last().is_some_and(|l| l != "..") => {
                out.pop();
            }
            "" if i > 0 => {}
            c if win && i == 0 && c.ends_with(':') => out.push(c.to_ascii_lowercase()),
            c => out.push(c.to_string()),
        }
    }
    (win, out)
}

/// `file` without the prefix `root`, if it starts with it. Either side may use `\` or `/`, have a
/// drive letter in any case, a `\\?\` prefix or `.`/`..` components. Windows paths compare
/// case-insensitively and are shown with `/`.
fn relative(file: &str, root: &str) -> String {
    let ((fw, f), (rw, r)) = (components(file), components(root));
    let same = |a: &String, b: &String| {
        if fw {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    if fw == rw && f.len() > r.len() && r.iter().zip(&f).all(|(a, b)| same(a, b)) {
        f[r.len()..].join("/")
    } else {
        file.to_string()
    }
}

/// Write the HTML report into `dir`; every source file the database names must be readable.
/// Pages show the paths without the prefix `root` (default the current directory).
fn write_html(
    dir: &PathBuf,
    root: Option<PathBuf>,
    db: &Database,
    excluded: &Excluded,
) -> Result<(), Box<Diagnostic>> {
    let mut sources = std::collections::BTreeMap::new();
    for f in yavs_cov::html::files(db, excluded) {
        let t = std::fs::read_to_string(f).map_err(|e| {
            Box::new(
                Diagnostic::error(SOURCE_ERROR, format!("cannot read source file `{f}`"))
                    .with_note(e.to_string())
                    .with_help(
                        "run `yavs cov report --html` where the paths in the database resolve",
                    ),
            )
        })?;
        sources.insert(f.to_string(), t);
    }
    let root = match root {
        Some(r) => std::path::absolute(&r).map_err(|e| {
            Box::new(
                Diagnostic::error(SOURCE_ERROR, format!("cannot resolve `{}`", r.display()))
                    .with_note(e.to_string()),
            )
        })?,
        None => std::env::current_dir().map_err(|e| {
            Box::new(
                Diagnostic::error(SOURCE_ERROR, "cannot find the current directory")
                    .with_note(e.to_string())
                    .with_help("pass `--root DIR`"),
            )
        })?,
    };
    let mut db = db.clone();
    let mut excluded = excluded.clone();
    let shown = db
        .items
        .values_mut()
        .chain(excluded.iter_mut().map(|x| &mut x.1));
    let root = root.display().to_string();
    shown.for_each(|e| e.file = relative(&e.file, &root));
    let sources = sources
        .into_iter()
        .map(|(f, t)| (relative(&f, &root), t))
        .collect();
    let (db, excluded) = (&db, &excluded);
    std::fs::create_dir_all(dir).map_err(|e| {
        Box::new(
            Diagnostic::error(WRITE_ERROR, format!("cannot write `{}`", dir.display()))
                .with_note(e.to_string()),
        )
    })?;
    for (name, text) in yavs_cov::html::pages(db, excluded, &sources) {
        write(&dir.join(name), text)?;
    }
    Ok(())
}

fn report(
    out: &mut Out,
    db: Database,
    cobertura: Option<PathBuf>,
    html: Option<(PathBuf, Option<PathBuf>)>,
    exclude: Option<PathBuf>,
    uncovered: Option<UncoveredFormat>,
    tests: bool,
) -> Result<(), Box<Diagnostic>> {
    let (db, excluded) = excluded(out, db, exclude)?;
    if let Some(f) = uncovered {
        match f {
            UncoveredFormat::Text => print!("{}", db.uncovered()),
            UncoveredFormat::Yaml => print!("{}", db.uncovered_yaml()),
        }
    } else if tests {
        print!("{}", db.hits_by_test());
    } else {
        print!("{}", db.summary());
        print!("{}", yavs_cov::exclude::excluded_text(&excluded));
    }
    if let Some((d, root)) = html {
        write_html(&d, root, &db, &excluded)?;
    }
    match cobertura {
        Some(p) => write(&p, db.cobertura()),
        None => Ok(()),
    }
}

pub fn run(c: Command, format: MessageFormat, color: bool) -> ExitCode {
    let mut out = Out::new(format, color);
    let r = match c {
        Command::Merge { output, inputs } => {
            merged(&inputs).and_then(|db| write(&output, db.write()))
        }
        Command::Report {
            cobertura,
            html,
            root,
            exclude,
            uncovered,
            tests,
            warn,
            inputs,
        } => out
            .set_warn(&warn)
            .and_then(|()| merged(&inputs))
            .and_then(|db| {
                let html = html.map(|d| (d, root));
                report(&mut out, db, cobertura, html, exclude, uncovered, tests)
            }),
        Command::Rank {
            exclude,
            warn,
            inputs,
        } => out
            .set_warn(&warn)
            .and_then(|()| merged(&inputs))
            .and_then(|db| excluded(&mut out, db, exclude))
            .map(|(db, _)| print!("{}", db.rank())),
    };
    match r {
        Ok(()) => out.finish(u8::from(out.errors > 0)),
        Err(d) => {
            out.emit(&SourceMap::default(), &d);
            out.finish(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::relative;

    #[test]
    fn relative_unix() {
        assert_eq!(relative("/a/b/c.vhd", "/a/b"), "c.vhd");
        assert_eq!(relative("/a/b/d/c.vhd", "/a/x/../b/./"), "d/c.vhd");
        assert_eq!(relative("/a/bc/c.vhd", "/a/b"), "/a/bc/c.vhd");
        assert_eq!(relative("/A/b/c.vhd", "/a/b"), "/A/b/c.vhd");
        assert_eq!(relative("rel/c.vhd", "/a"), "rel/c.vhd");
    }

    #[test]
    fn relative_windows() {
        let want = "tests/cli/c.vhd";
        // Backslashes, a root with `..` and mixed separators, as cargo builds it.
        let root = r"D:\a\yavs\yavs\crates\yavs/../../tests";
        assert_eq!(
            relative(r"D:\a\yavs\yavs\tests\cli\c.vhd", root),
            "cli/c.vhd"
        );
        assert_eq!(
            relative(r"D:\a\yavs\yavs\tests\cli\c.vhd", r"D:\a\yavs\yavs"),
            want
        );
        // Drive letter and directory case, forward slashes, verbatim and UNC prefixes.
        assert_eq!(
            relative(r"d:/a/YAVS/yavs/tests/cli/c.vhd", r"D:\a\yavs\yavs"),
            want
        );
        assert_eq!(
            relative(r"\\?\D:\a\yavs\yavs\tests\cli\c.vhd", r"D:\a\yavs\yavs"),
            want
        );
        assert_eq!(
            relative(r"D:\a\yavs\yavs\tests\cli\c.vhd", r"\\?\d:\a\yavs\yavs"),
            want
        );
        assert_eq!(relative(r"\\?\UNC\srv\sh\x\c.vhd", r"\\srv\sh"), "x/c.vhd");
        // Another drive or a sibling directory stays absolute.
        assert_eq!(relative(r"E:\a\c.vhd", r"D:\a"), r"E:\a\c.vhd");
        assert_eq!(relative(r"D:\ab\c.vhd", r"D:\a"), r"D:\ab\c.vhd");
        // A Windows file against a Unix root is not relative.
        assert_eq!(relative(r"D:\a\c.vhd", "/a"), r"D:\a\c.vhd");
    }
}
