//! Clock-edge idioms the analyser recognises (`Db::edge_shape`, #632) and the look-alikes it
//! must not, and `not s'stable` read as `s'event` without an implicit signal.

use std::path::PathBuf;

use yavs_sem::decl::{DeclKind, ObjRole};
use yavs_sem::edge::EdgeShape;
use yavs_sem::hir::{ExprKind, StmtKind};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

const SRC: &str = "
library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture a of e is
  signal clk, clk2 : std_logic := '0';
  signal cu : std_ulogic := '0';
  signal cb : bit := '0';
  signal cbool : boolean := false;
  signal ck : integer := 0;
  signal v : std_logic_vector(3 downto 0) := (others => '0');
  signal q : std_logic := '0';
  function rising_edge(signal s : integer) return boolean is begin return s'event; end;
begin
  rise:    process (clk)   begin if rising_edge(clk)  then q <= not q; end if; end process;
  fall:    process (clk)   begin if falling_edge(clk) then q <= not q; end if; end process;
  rise_u:  process (cu)    begin if rising_edge(cu)   then q <= not q; end if; end process;
  rise_b:  process (cb)    begin if rising_edge(cb)   then q <= not q; end if; end process;
  rise_o:  process (cbool) begin if rising_edge(cbool) then q <= not q; end if; end process;
  ev1:     process (clk)   begin if clk'event and clk = '1' then q <= not q; end if; end process;
  ev1r:    process (clk)   begin if clk = '1' and clk'event then q <= not q; end if; end process;
  ev0:     process (clk)   begin if clk'event and clk = '0' then q <= not q; end if; end process;
  evb:     process (cb)    begin if cb'event and cb = '1' then q <= not q; end if; end process;
  evbool:  process (cbool) begin if cbool'event and cbool then q <= not q; end if; end process;
  evnot:   process (cbool) begin if cbool'event and not cbool then q <= not q; end if; end process;
  evelem:  process (v)     begin if v(2)'event and v(2) = '1' then q <= not q; end if; end process;
  nstab:   process (clk)   begin if not clk'stable and clk = '1' then q <= not q; end if; end process;
  nstab_b: process (cbool) begin if not cbool'stable and cbool then q <= not q; end if; end process;
  level:   process (clk)   begin if clk = '1' then q <= not q; end if; end process;
  evonly:  process (clk)   begin if clk'event then q <= not q; end if; end process;
  other:   process (clk, clk2) begin if clk'event and clk2 = '1' then q <= not q; end if; end process;
  evh:     process (clk)   begin if clk'event and clk = 'H' then q <= not q; end if; end process;
  user:    process (ck)    begin if rising_edge(ck) then q <= not q; end if; end process;
  stab5:   process (clk)   begin if not clk'stable(5 ns) and clk = '1' then q <= not q; end if; end process;
end;
";

// One directory per test and process: parallel tests would read a half written shared file.
fn analyze(test: &str) -> Db {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("edge_shapes_{}_{test}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("e.vhd");
    std::fs::write(&path, SRC).expect("write");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    let r = db.analyze_file("work", &path, LangRev::Vhdl2008);
    assert!(
        r.is_ok() && db.diagnostics().is_empty(),
        "{:?}",
        db.diagnostics()
    );
    db
}

/// The edge shape of the `if` condition of the process labelled `label`.
fn shape(db: &Db, label: &str) -> Option<EdgeShape> {
    let cond = cond_of(db, label);
    db.edge_shape(cond)
}

fn cond_of(db: &Db, label: &str) -> yavs_sem::ids::ExprId {
    for (_, s) in db.stmts.iter() {
        if let StmtKind::Process { body, .. } = &s.kind
            && s.label.is_some_and(|l| l.text.as_str() == label)
            && let StmtKind::If { branches, .. } = &db.stmts[body[0]].kind
        {
            return branches[0].0;
        }
    }
    panic!("no process {label}");
}

#[test]
fn recognised_forms() {
    let db = analyze("recognised_forms");
    for (label, rising, legacy) in [
        ("rise", true, false),
        ("fall", false, false),
        ("rise_u", true, false),
        ("rise_b", true, false),
        ("rise_o", true, false),
        ("ev1", true, true),
        ("ev1r", true, true),
        ("ev0", false, true),
        ("evb", true, true),
        ("evbool", true, true),
        ("evnot", false, true),
        ("evelem", true, true),
        ("nstab", true, true),
        ("nstab_b", true, true),
    ] {
        let s = shape(&db, label).unwrap_or_else(|| panic!("{label} is an edge test"));
        assert_eq!((s.rising, s.legacy), (rising, legacy), "{label}");
        assert_eq!(s.other.is_some(), legacy, "{label}");
        if let Some(o) = s.other {
            assert!(db.same_name(s.sig, o), "{label}: one signal");
        }
    }
}

#[test]
fn look_alikes_are_not_edge_tests() {
    let db = analyze("look_alikes_are_not_edge_tests");
    for label in ["level", "evonly", "evh", "user", "stab5"] {
        assert_eq!(shape(&db, label), None, "{label}");
    }
    // Two signals: the shape is there, the caller sees that the names differ.
    let s = shape(&db, "other").expect("two names");
    assert!(!db.same_name(s.sig, s.other.expect("other")));
}

#[test]
fn not_stable_declares_no_implicit_signal() {
    let db = analyze("not_stable_declares_no_implicit_signal");
    // `not clk'stable(5 ns)` keeps its implicit signal, the two without a time do not.
    let implicit = db
        .decls
        .iter()
        .filter(
            |(_, d)| matches!(&d.kind, DeclKind::Object(o) if o.role == ObjRole::ImplicitSignal),
        )
        .count();
    let in_scopes: usize = db
        .scopes
        .iter()
        .map(|(_, s)| {
            s.order
                .iter()
                .filter(|&&d| {
                    matches!(&db.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::ImplicitSignal)
                })
                .count()
        })
        .sum();
    assert_eq!(implicit, 3, "one per occurrence made");
    assert_eq!(in_scopes, 1, "only the one with a time is declared");
    let c = cond_of(&db, "nstab");
    assert!(matches!(db.exprs[c].kind, ExprKind::Call { .. }));
}
