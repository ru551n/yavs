//! `Db::facts()`: which signal objects have `'last_event` or `'last_value` applied to a part of
//! them or to something the elaborator cannot trace (#519): the kernel keeps the event history
//! of single elements of exactly those wide signals.

use std::path::PathBuf;

use yavs_sem::decl::Designator;
use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// `(history_all, names of the objects with a part, names of the whole ones)` for the statements
/// `stmts` of a process of an architecture with signals `a` and `b`.
fn facts(test: &str, decls: &str, stmts: &str) -> (bool, Vec<String>, Vec<String>) {
    let src = format!(
        "library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture t of e is
  signal a, b : std_logic_vector(99 downto 0);
  signal i : integer range 0 to 99;
  {decls}
begin
  process
    variable t1 : time;
    variable s : std_logic;
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
    db.analyze_file("work", &path, LangRev::Vhdl2008)
        .expect("analysis");
    assert!(db.diagnostics().is_empty(), "{:?}", db.diagnostics());
    let f = db.facts();
    let name = |d| match &db.decls[d].name {
        Designator::Ident(i) => i.text.as_str().to_string(),
        n => format!("{n:?}"),
    };
    let mut parts: Vec<String> = f.history_parts.iter().map(|&d| name(d)).collect();
    let mut wholes: Vec<String> = f.history_wholes.iter().map(|&d| name(d)).collect();
    // The standard packages' own uses are scalar 'last_value of parameters: no fact.
    parts.sort();
    wholes.sort();
    (f.history_all, parts, wholes)
}

#[test]
fn whole_and_part_prefixes() {
    let (all, parts, wholes) = facts("whole", "", "t1 := a'last_event; w := b'last_value;");
    assert!(!all);
    assert!(parts.is_empty(), "{parts:?}");
    assert_eq!(wholes, ["a", "b"]);
    let (all, parts, wholes) = facts(
        "part",
        "",
        "t1 := a(3)'last_event; t1 := b(7 downto 0)'last_event; t1 := a(i)'last_event;",
    );
    assert!(!all && wholes.is_empty());
    assert_eq!(parts, ["a", "b"]);
}

#[test]
fn a_scalar_last_value_needs_no_fact_and_an_unnamed_prefix_asks_everything() {
    let (all, parts, wholes) = facts("scalar", "", "s := a(3)'last_value; s := a(i)'last_value;");
    assert!(!all && parts.is_empty() && wholes.is_empty());
    // A signal parameter may be bound to any signal; so may an alias of nothing traceable.
    let decls = "procedure p (signal x : in std_logic_vector) is variable t : time; begin t := x'last_event; end;";
    assert!(facts("param", decls, "p(a);").0);
    // Aliases are followed to their object.
    let decls = "alias al : std_logic_vector(7 downto 0) is a(7 downto 0); alias all2 is b;";
    let (all, parts, wholes) = facts(
        "alias",
        decls,
        "t1 := al'last_event; t1 := all2'last_event;",
    );
    assert!(!all);
    assert_eq!(
        (parts, wholes),
        (vec!["a".to_string()], vec!["b".to_string()])
    );
}

/// `Facts::history_reads`: the names of the objects a history attribute or a signal parameter
/// reaches, and `history_reads_all`.
fn reads(test: &str, decls: &str, stmts: &str) -> (bool, Vec<String>) {
    let src = format!(
        "library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture t of e is
  signal a, b, c : std_logic_vector(99 downto 0);
  {decls}
begin
  process
    variable t1 : time;
    variable s : std_logic;
    variable w : std_logic_vector(99 downto 0);
    variable q : boolean;
  begin
    {stmts}
    wait;
  end process;
end;"
    );
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("reads_{test}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("f.vhd");
    std::fs::write(&path, src).expect("write");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    db.analyze_file("work", &path, LangRev::Vhdl2008)
        .expect("analysis");
    assert!(db.diagnostics().is_empty(), "{:?}", db.diagnostics());
    let f = db.facts();
    let name = |d| match &db.decls[d].name {
        Designator::Ident(i) => i.text.as_str().to_string(),
        n => format!("{n:?}"),
    };
    let mut v: Vec<String> = f.history_reads.iter().map(|&d| name(d)).collect();
    v.sort();
    (f.history_reads_all, v)
}

#[test]
fn every_history_attribute_reads_its_object() {
    for (i, stmts) in [
        "q := a'event;",
        "q := a(3)'event;",
        "q := a(9 downto 2)'active;",
        "t1 := a'last_event;",
        "t1 := a(4)'last_active;",
        "w := a'last_value;",
        "s := a(5)'last_value;",
        "q := a'stable;",
        "q := a'quiet(1 ns);",
        "q := a'transaction = '1';",
        "w := a'delayed(1 ns);",
    ]
    .iter()
    .enumerate()
    {
        let (all, v) = reads(&format!("attr{i}"), "", stmts);
        assert!(!all, "{stmts}");
        assert_eq!(v, ["a"], "{stmts}");
    }
    let (all, v) = reads("none", "", "w := a; s := b(3); q := a'length = 3;");
    assert!(!all && v.is_empty(), "{v:?}");
}

#[test]
fn a_signal_actual_and_an_alias_read_their_object() {
    let p = "procedure p (signal s : in std_logic) is begin null; end;";
    assert_eq!(reads("arg", p, "p(a(3));"), (false, vec!["a".to_string()]));
    let (all, v) = reads("edge", "", "q := rising_edge(b(70));");
    assert!(!all);
    assert_eq!(v, ["b"]);
    let al = "alias x : std_logic_vector(7 downto 0) is c(9 downto 2);";
    let (all, v) = reads("alias", al, "q := x'event;");
    assert!(!all);
    assert_eq!(v, ["c"]);
    // An attribute of a signal parameter is the caller's: only the actual is noted.
    let p = "procedure p (signal s : in std_logic_vector) is variable t : time; begin t := s'last_event; end;";
    let (all, v) = reads("param", p, "p(a);");
    assert!(!all);
    assert_eq!(v, ["a"]);
}

#[test]
fn an_external_name_reads_everything() {
    let al = "alias x is << signal .e.a : std_logic_vector(99 downto 0) >>;";
    let (all, _) = reads("ext", al, "q := x'event;");
    assert!(all);
}
