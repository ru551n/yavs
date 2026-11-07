//! `yavs analyze` of Verilog files through the front end: units in the library, macros across files,
//! includes, protected IP and name clashes with VHDL (#776, M0).

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};
use yavs_lib::{Libraries, Library, ModuleLookup, UnitKind, UnitRef, closure};

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

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

#[test]
fn modules_are_recorded_with_their_case_and_include_hashes() {
    let t = tempdir("verilog-units");
    std::fs::create_dir(t.join("inc")).unwrap();
    write(&t, "inc/defs.vh", "`define W 8\n");
    write(
        &t,
        "a.v",
        "`timescale 1ns/1ps\n`include \"defs.vh\"\nmodule Top; wire [`W-1:0] x; endmodule\nmodule top; endmodule\n",
    );
    let lib = format!("--work=work:{}", t.join("lib").display());
    let o = yavs(&t, &["analyze", &lib, "-I", "inc", "a.v"]);
    assert!(o.status.success(), "{}", text(&o));
    let work = Library::new("work", t.join("lib"));
    let m = work.read().unwrap();
    // Both spellings are units; the exact one is found first.
    let key = |n: &str| match m.find_module(n) {
        ModuleLookup::Found(u) => u.key.to_string(),
        ModuleLookup::Ambiguous(_) => "ambiguous".into(),
        ModuleLookup::Missing => "missing".into(),
    };
    assert_eq!(
        (key("Top"), key("top"), key("TOP")),
        ("Top".into(), "top".into(), "ambiguous".into())
    );
    let recs = m.records(&m.files().next().unwrap()).unwrap();
    assert!(recs[0].context.is_some());
    assert!(recs[0].includes[0].0.ends_with("defs.vh"));
    // The closure of the module checks the include like the source.
    let mut libs = Libraries::new();
    libs.add("work", t.join("lib"));
    let top = UnitRef {
        library: "work".into(),
        kind: UnitKind::Module,
        key: "Top".into(),
    };
    assert!(closure(&libs, &top).unwrap()[0].context.contains("defs.vh"));
    write(&t, "inc/defs.vh", "`define W 16\n");
    assert_eq!(closure(&libs, &top).unwrap_err().code, "L0034");
}

#[test]
fn macros_persist_from_one_file_of_a_call_to_the_next() {
    let t = tempdir("verilog-macros");
    write(&t, "defs.vh", "`define W 8\n");
    write(&t, "m.v", "module m; wire [`W-1:0] x; endmodule\n");
    let lib = format!("--work=work:{}", t.join("lib").display());
    let o = yavs(&t, &["analyze", &lib, "defs.vh", "m.v"]);
    assert!(o.status.success(), "{}", text(&o));
    // Alone, the macro is not defined; with -D it is.
    let o = yavs(&t, &["analyze", &lib, "m.v"]);
    assert!(!o.status.success());
    let o = yavs(&t, &["analyze", &lib, "-D", "W=8", "m.v"]);
    assert!(o.status.success(), "{}", text(&o));
}

#[test]
fn protected_ip_is_recorded_by_name() {
    let t = tempdir("verilog-protected");
    write(
        &t,
        "sip.vp",
        "module SIP_CELL (input a, output b);\n`pragma protect begin_protected\n`pragma protect key_keyowner=\"Vendor\"\nYWJj\n`pragma protect end_protected\nendmodule\n",
    );
    let lib = format!("--work=work:{}", t.join("lib").display());
    let o = yavs(&t, &["analyze", &lib, "sip.vp"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("W4007"), "{}", text(&o));
    let work = Library::new("work", t.join("lib"));
    let m = work.read().unwrap();
    // Recorded with a warning, so a directory of vendor files analyses.
    let ModuleLookup::Found(u) = m.find_module("sip_cell") else {
        panic!("not recorded: {}", text(&o))
    };
    let recs = m.records(&m.file(u.file).unwrap()).unwrap();
    assert!(recs[0].encrypted);
}

#[test]
fn a_module_and_a_vhdl_entity_may_not_share_a_name() {
    let t = tempdir("verilog-clash");
    write(&t, "e.vhd", "entity Cell is end entity;\n");
    write(&t, "c.v", "module cell; endmodule\n");
    let lib = format!("--work=work:{}", t.join("lib").display());
    assert!(yavs(&t, &["analyze", &lib, "e.vhd"]).status.success());
    let o = yavs(&t, &["analyze", &lib, "c.v"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o).contains("L0035"), "{}", text(&o));
}
