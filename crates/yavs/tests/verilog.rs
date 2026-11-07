//! What the command line does around Verilog units that a library holds, without the front end:
//! `elab` stops at one (`E4500`), `lint` leaves it alone (#776, M0).

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};
use yavs_lib::{Library, UnitKind, UnitRecord, VERILOG_STD, hash_content};

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A library with the Verilog module `Cell` in `c.v`, recorded as the front end would.
fn seeded(t: &Path) -> Library {
    let src = "module Cell; endmodule\n";
    std::fs::write(t.join("c.v"), src).unwrap();
    let work = Library::new("work", t.join("lib"));
    let unit = UnitRecord::verilog(UnitKind::Module, "Cell", VERILOG_STD);
    work.update_file(
        VERILOG_STD,
        &t.join("c.v"),
        &hash_content(src.as_bytes()),
        vec![unit],
    )
    .unwrap();
    work
}

#[test]
fn elab_stops_at_verilog() {
    let t = tempdir("verilog-elab");
    seeded(&t);
    let lib = format!("--work=work:{}", t.join("lib").display());
    // The module is found by its name as written, and stopped at.
    let o = yavs(&t, &["elab", &lib, "Cell"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).contains("E4500"), "{}", text(&o));
    // Another spelling is not a module: a top level that is not there.
    let o = yavs(&t, &["elab", &lib, "cell"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).contains("E0104"), "{}", text(&o));
}

#[test]
fn lint_does_not_read_verilog_as_vhdl() {
    let t = tempdir("verilog-lint");
    let work = seeded(&t);
    let src = "entity e is end entity;\n";
    std::fs::write(t.join("e.vhd"), src).unwrap();
    work.update_file(
        "2008",
        &t.join("e.vhd"),
        &hash_content(src.as_bytes()),
        vec![UnitRecord::new(UnitKind::Entity, "e", "2008")],
    )
    .unwrap();
    let o = yavs(
        &t,
        &["lint", &format!("--work=work:{}", t.join("lib").display())],
    );
    assert!(!text(&o).contains("c.v"), "{}", text(&o));
    assert!(o.status.code().is_some_and(|c| c < 2), "{}", text(&o));
}
