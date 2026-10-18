//! `cargo run -p rsim-sem --example analyze -- [--std 2019] [--design-std 2019] FILE|@lib.pkg...`:
//! analyze files into `work` and print the diagnostics (development aid until
//! `rsim analyze` exists). `RAW=1` prints the diagnostic structs instead.

use rsim_diag::{MessageFormat, render};
use rsim_sem::{Db, LangRev, Options, default_stdlib};

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
    for f in &args {
        // `@lib.pkg` loads a package and its body from the library instead of a file.
        if let Some((lib, pkg)) = f.strip_prefix('@').and_then(|u| u.split_once('.')) {
            if db.load_primary(lib, pkg).is_ok() {
                let _ = db.load_body(lib, pkg);
            }
            continue;
        }
        let _ = db.analyze_file("work", f.as_ref(), file_rev);
    }
    for d in db.diagnostics() {
        if std::env::var_os("RAW").is_some() {
            eprintln!("{d:?}");
        } else {
            eprint!("{}", render(&db.sources, d, MessageFormat::Human, false));
        }
    }
    std::process::exit(i32::from(db.error_count() > 0));
}
