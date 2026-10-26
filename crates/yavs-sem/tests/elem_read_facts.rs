//! `Db::facts()`: which signal objects have one element read (#734): the kernel keeps byte codes
//! for those only.

use std::path::PathBuf;

use yavs_sem::decl::Designator;
use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// `(elem_reads_all, names of the objects with an element read)` for the statements `stmts` of
/// a process of an architecture with signals `a`, `b` and `c`.
fn facts(test: &str, decls: &str, stmts: &str) -> (bool, Vec<String>) {
    let src = format!(
        "library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture t of e is
  signal a, b, c : std_logic_vector(99 downto 0);
  signal i : integer range 0 to 99;
  signal s : std_logic;
  type rec is record f : std_logic; g : std_logic_vector(3 downto 0); end record;
  signal r : rec;
  {decls}
begin
  process
    variable v : std_logic;
    variable w : std_logic_vector(99 downto 0);
  begin
    {stmts}
    wait;
  end process;
end;"
    );
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{test}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("f.vhd");
    std::fs::write(&path, src).expect("write");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    let r = db.analyze_file("work", &path, LangRev::Vhdl2008);
    assert!(r.is_ok(), "{:?}", db.diagnostics());
    assert!(db.diagnostics().is_empty(), "{:?}", db.diagnostics());
    let f = db.facts();
    let mut reads: Vec<String> = f
        .elem_reads
        .iter()
        .map(|&d| match &db.decls[d].name {
            Designator::Ident(i) => i.text.as_str().to_string(),
            n => format!("{n:?}"),
        })
        .collect();
    reads.sort();
    (f.elem_reads_all, reads)
}

#[test]
fn an_element_read_marks_its_object_and_a_whole_or_slice_read_does_not() {
    let (all, reads) = facts(
        "reads",
        "",
        "v := a(3); v := b(i); w := c; w(7 downto 0) := a(7 downto 0); v := r.f; v := r.g(1);",
    );
    assert!(!all);
    assert_eq!(reads, ["a", "b", "r"]);
    let (all, reads) = facts(
        "whole",
        "",
        "w := a; w(3 downto 0) := b(3 downto 0); v := s;",
    );
    assert!(!all);
    assert!(reads.is_empty(), "{reads:?}");
}

#[test]
fn an_assignment_target_is_not_a_read() {
    let (all, reads) = facts(
        "target",
        "",
        "a(3) <= '1'; b(i) <= '0'; c(2) <= force '1'; c(2) <= release; r.g(0) <= '1'; v := b(1);",
    );
    assert!(!all);
    assert_eq!(reads, ["b"]);
}

#[test]
fn aliases_are_followed_and_a_signal_parameter_asks_everything() {
    let (all, reads) = facts(
        "alias",
        "alias al : std_logic is a(5); alias bl : std_logic_vector(7 downto 0) is b(7 downto 0);",
        "v := al; v := bl(2);",
    );
    assert!(!all);
    assert_eq!(reads, ["a", "b"]);
    let decls = "procedure p (signal x : in std_logic_vector) is variable t : std_logic; begin t := x(2); end;";
    assert!(facts("param", decls, "null;").0);
}
