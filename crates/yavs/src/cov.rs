//! `yavs cov merge`, `yavs cov report` and `yavs cov rank` (docs/cli-contract.md section 5).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use yavs_cov::Database;
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};

use crate::analyze::Out;

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
        /// Leave out the items an exclusion file lists (rows pasted from `--uncovered`).
        #[arg(long, value_name = "FILE")]
        exclude: Option<PathBuf>,
        /// Print only the items that never ran, as exclusion rows.
        #[arg(long)]
        uncovered: bool,
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

/// `db` without the items the exclusion file lists, and those items.
fn excluded(
    out: &mut Out,
    db: Database,
    exclude: Option<PathBuf>,
) -> Result<(Database, Vec<(yavs_cov::Key, yavs_cov::Entry, String)>), Box<Diagnostic>> {
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
    let rows = std::fs::read_to_string(p)
        .map_err(|e| e.to_string())
        .and_then(|t| yavs_cov::exclude::parse(&t))
        .map_err(fail)?;
    let a = yavs_cov::exclude::apply(&db, &rows, &p.display().to_string());
    for w in &a.warnings {
        out.emit(&SourceMap::default(), w);
    }
    Ok((a.kept, a.excluded))
}

fn report(
    out: &mut Out,
    db: Database,
    cobertura: Option<PathBuf>,
    exclude: Option<PathBuf>,
    uncovered: bool,
    tests: bool,
) -> Result<(), Box<Diagnostic>> {
    let (db, excluded) = excluded(out, db, exclude)?;
    if uncovered {
        print!("{}", db.uncovered());
    } else if tests {
        print!("{}", db.hits_by_test());
    } else {
        print!("{}", db.summary());
        print!("{}", yavs_cov::exclude::excluded_text(&excluded));
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
            exclude,
            uncovered,
            tests,
            warn,
            inputs,
        } => out
            .set_warn(&warn)
            .and_then(|()| merged(&inputs))
            .and_then(|db| report(&mut out, db, cobertura, exclude, uncovered, tests)),
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
