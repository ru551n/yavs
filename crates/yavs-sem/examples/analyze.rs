//! `cargo run -p yavs-sem --example analyze -- [--std 2019] [--design-std 2019] FILE|@lib.pkg...`:
//! analyze files into `work` and print the diagnostics (development aid until
//! `yavs analyze` exists). `RAW=1` prints the diagnostic structs instead. `LIB:FILE` analyzes
//! into library `LIB`; `SUMMARY=1` prints one `PASS|FAIL file codes` line per file.

use yavs_diag::{MessageFormat, render};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str| -> Option<LangRev> {
        let i = args.iter().position(|a| a == flag)?;
        let v = args.remove(i + 1);
        args.remove(i);
        Some(if v == "2019" {
            LangRev::Vhdl2019
        } else {
            LangRev::Vhdl2008
        })
    };
    let file_rev = take("--std").unwrap_or(LangRev::Vhdl2008);
    let design = take("--design-std").unwrap_or(file_rev);
    let mut db = Db::new(Options::new(design, default_stdlib()));
    let summary = std::env::var_os("SUMMARY").is_some();
    for a in &args {
        // `@lib.pkg` loads a package and its body from the library instead of a file.
        if let Some((lib, pkg)) = a.strip_prefix('@').and_then(|u| u.split_once('.')) {
            if db.load_primary(lib, pkg).is_ok() {
                let _ = db.load_body(lib, pkg);
            }
            continue;
        }
        let (lib, f) = a.split_once(':').unwrap_or(("work", a));
        let before = db.diagnostics().len();
        let r = db.analyze_file(lib, f.as_ref(), file_rev);
        if summary {
            let codes: Vec<&str> = db.diagnostics()[before..].iter().map(|d| d.code).collect();
            let v = if r.is_ok() { "PASS" } else { "FAIL" };
            println!("{v} {f} {}", codes.join(" "));
        }
    }
    for d in db.diagnostics() {
        if summary {
            break;
        }
        if std::env::var_os("RAW").is_some() {
            eprintln!("{d:?}");
        } else {
            eprint!("{}", render(&db.sources, d, MessageFormat::Human, false));
        }
    }
    std::process::exit(i32::from(db.error_count() > 0));
}
