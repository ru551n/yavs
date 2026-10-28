//! `Db::facts()`: which signal objects have one element read (#734): the kernel keeps byte codes
//! for those only.

use std::path::PathBuf;

use yavs_sem::decl::Designator;
use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// `(elem_reads_all, names of the objects with an element read)` for the statements `stmts` of
/// a process of an architecture with signals `a`, `b` and `c`.
fn facts(test: &str, decls: &str, stmts: &str) -> (bool, Vec<String>) {
    facts_in(test, "", decls, "", stmts)
}

/// [`facts`] with design units `units` before the entity and concurrent statements `conc`.
fn facts_in(test: &str, units: &str, decls: &str, conc: &str, stmts: &str) -> (bool, Vec<String>) {
    let src = format!(
        "{units}
library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture t of e is
  signal a, b, c : std_logic_vector(99 downto 0);
  signal i : integer range 0 to 99;
  signal s : std_logic;
  type rec is record f : std_logic; g : std_logic_vector(3 downto 0); end record;
  signal r : rec;
  {decls}
begin
  {conc}
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
    // A signal parameter asks for the signals passed to it, not for everything.
    assert_eq!(facts("param", decls, "null;"), (false, vec![]));
    assert_eq!(
        facts("param_call", decls, "p(a); p(b(7 downto 0));"),
        (false, vec!["a".into(), "b".into()])
    );
}

#[test]
fn a_signal_passed_on_is_followed() {
    let decls = "procedure p (signal x : in std_logic_vector) is variable t : std_logic; begin t := x(2); end;
  procedure q (signal y : in std_logic_vector) is begin p(y); end;
  procedure rr (signal z : in std_logic_vector) is variable t : std_logic; begin t := z(1) and z(0); end;
  procedure s2 (signal w : in std_logic_vector) is begin null; end;";
    assert_eq!(
        facts("pass", decls, "q(a); s2(b); rr(c);"),
        (false, vec!["a".into(), "c".into()])
    );
    assert_eq!(facts("pass_none", decls, "s2(b);"), (false, vec![]));
}

#[test]
fn a_body_completing_a_spec_reads_through_the_spec() {
    let decls = "procedure p (signal x : in std_logic_vector; signal y : in std_logic_vector);
  procedure p (signal x : in std_logic_vector; signal y : in std_logic_vector) is variable t : std_logic; begin t := y(2); end;";
    assert_eq!(
        facts("spec_body", decls, "p(a, b); p(c, c);"),
        (false, vec!["b".into(), "c".into()])
    );
}

#[test]
fn an_external_name_passed_to_a_reading_parameter_asks_everything() {
    let decls = "procedure p (signal x : in std_logic_vector) is variable t : std_logic; begin t := x(2); end;";
    let ext = "p(<<signal .e.a : std_logic_vector(99 downto 0)>>);";
    assert!(facts("ext_read", decls, ext).0);
    let decls = "procedure p (signal x : in std_logic_vector) is begin null; end;";
    assert!(!facts("ext_noread", decls, ext).0);
}

const SUB: &str = "library ieee; use ieee.std_logic_1164.all;
entity sub is port (x : in std_logic; y : out std_logic; z : in std_logic_vector(3 downto 0)); end;
architecture t of sub is
  signal q : std_logic;
begin
  process (x) begin q <= x; end process;
  y <= '1';
  process (z) begin q <= z(1); end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity sub2 is port (x : in std_logic; y : out std_logic); end;
architecture t of sub2 is begin y <= '0'; end;
";

#[test]
fn a_port_map_actual_is_a_connection_and_a_scalar_port_read_whole_is_the_read() {
    let conc = "u1: entity work.sub port map (x => a(3), y => b(4), z => c(3 downto 0));
  u2: entity work.sub2 port map (x => a(5), y => b(6));";
    let (all, reads) = facts_in("portmap", SUB, "", conc, "null;");
    assert!(!all);
    // The ports are the reads, not the actuals: sub's x (`q <= x`; its sensitivity entry is
    // none) is an element of a when collapsed, z(1) an element of c; y is only driven and
    // sub2 reads nothing.
    assert_eq!(reads, ["x", "z"]);
}

#[test]
fn a_sensitivity_entry_is_no_read() {
    let conc = "process (a(3), c) begin null; end process;";
    let (all, reads) = facts_in(
        "sens",
        "",
        "",
        conc,
        "wait on b(2); wait until a(7) = '1'; v := s;",
    );
    assert!(!all);
    assert_eq!(reads, ["a"]);
}
