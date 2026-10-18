//! Unit tests of generic.rs (sem.md section 5): instantiation by cloning,
//! instance identity, formal mapping, defaults, diagnostics.

use super::*;
use crate::stat::ConstValue;
use crate::{LangRev, Options, default_stdlib};

/// Analyze `src` as one file into `work`; scratch in a temporary directory.
fn analyze(rev: LangRev, src: &str) -> Db {
    let tmp = tempfile::tempdir().expect("scratch dir");
    let path = tmp.path().join("t.vhd");
    std::fs::write(&path, src).expect("write scratch file");
    let mut db = Db::new(Options::new(rev, default_stdlib()));
    let _ = db.analyze_file("work", &path, rev);
    db
}

fn codes(db: &Db) -> Vec<&'static str> {
    db.diagnostics().iter().map(|d| d.code).collect()
}

fn clean(src: &str) -> Db {
    let db = analyze(LangRev::Vhdl2008, src);
    let text: Vec<String> = db
        .diagnostics()
        .iter()
        .map(|d| yavs_diag::render(&db.sources, d, yavs_diag::MessageFormat::Short, false))
        .collect();
    assert!(text.is_empty(), "{text:?}");
    db
}

/// The declaration `name` of the region of library unit `unit` of `work`.
fn unit_decl(db: &Db, unit: &str, name: &str) -> DeclId {
    let s = unit_scope(db, unit);
    db.scopes[s].names[&Symbol::intern(name)][0]
}

fn unit_scope(db: &Db, unit: &str) -> ScopeId {
    db.units
        .iter()
        .find(|(_, u)| u.name.key.as_str() == unit && u.kind != UnitKind::PackageBody)
        .map(|(_, u)| u.scope)
        .expect("unit")
}

fn inst_of(db: &Db, unit: &str) -> InstId {
    match db
        .units
        .iter()
        .find(|(_, u)| u.name.key.as_str() == unit)
        .map(|(_, u)| &u.hir)
    {
        Some(UnitHir::PackageInst(i)) => *i,
        _ => panic!("{unit} is not a package instance"),
    }
}

const GEN: &str = "
package g is
  generic (n : integer := 1; type t);
  type rec is record a : t; end record;
  constant w : integer := n + 1;
  function f (a : t) return t;
  function eq (a, b : t) return boolean;
end package;
package body g is
  function f (a : t) return t is begin return a; end function;
  function eq (a, b : t) return boolean is begin return a = b; end function;
end package body;
package i1 is new work.g generic map (n => 8, t => integer);
package i2 is new work.g generic map (t => integer);
";

#[test]
fn instance_clones_the_visible_part_with_actuals() {
    let db = clean(GEN);
    let w = unit_decl(&db, "i1", "w");
    let DeclKind::Object(o) = &db.decls[w].kind else {
        panic!("w is an object")
    };
    // `n + 1` with `n => 8` folds again in the instance.
    assert_eq!(
        db.value(o.default.expect("default")),
        Some(&ConstValue::Int(9))
    );
    let w2 = unit_decl(&db, "i2", "w");
    let DeclKind::Object(o2) = &db.decls[w2].kind else {
        panic!("w is an object")
    };
    assert_eq!(
        db.value(o2.default.expect("default")),
        Some(&ConstValue::Int(2))
    );
    // The formal type is the actual: `f (a : integer) return integer`.
    let f = unit_decl(&db, "i1", "f");
    let s = db.decls[f].kind.subprogram().expect("f");
    let int = db.well_known.integer.expect("INTEGER");
    assert_eq!(db.base(s.params[0].subtype), int);
    assert_eq!(db.base(s.ret.expect("ret")), int);
    assert_eq!(
        s.mangled.as_str(),
        "work.i1.f(std.standard.integer)std.standard.integer"
    );
}

#[test]
fn every_instance_is_distinct_rule_4() {
    let db = clean(GEN);
    let r1 = unit_decl(&db, "i1", "rec");
    let r2 = unit_decl(&db, "i2", "rec");
    let rg = unit_decl(&db, "g", "rec");
    let ty = |d: DeclId| match db.decls[d].kind {
        DeclKind::Type(t) => t,
        _ => panic!("rec is a type"),
    };
    assert_ne!(ty(r1), ty(r2));
    assert_ne!(ty(r1), ty(rg));
    assert_eq!(crate::mangle::type_name(&db, ty(r1)), "work.i1.rec");
    assert_eq!(crate::mangle::type_name(&db, ty(r2)), "work.i2.rec");
    let (i1, i2) = (inst_of(&db, "i1"), inst_of(&db, "i2"));
    assert_eq!(db.insts[i1].mangled.as_str(), "work.i1");
    assert_ne!(db.insts[i1].mangled, db.insts[i2].mangled);
    // The record element is of the actual type.
    let TypeKind::Record { elems } = &db.types[ty(r1)].kind else {
        panic!("record")
    };
    assert_eq!(
        db.base(elems[0].subtype),
        db.well_known.integer.expect("INTEGER")
    );
}

#[test]
fn instance_body_maps_the_formal_eq_to_the_actual_predefined_eq() {
    let mut db = clean(GEN);
    let i1 = inst_of(&db, "i1");
    assert!(db.insts[i1].body.get().is_none(), "bodies are lazy");
    let body = db.instance_body(i1).expect("body").expect("has a body");
    assert_eq!(db.instance_body(i1).expect("again"), Some(body));
    let eq = unit_decl(&db, "i1", "eq");
    let Some(Body::Hir { stmts, .. }) = db.decls[eq].kind.subprogram().map(|s| &s.body) else {
        panic!("eq has its cloned body")
    };
    let StmtKind::Return(Some(e)) = db.stmts[stmts[0]].kind else {
        panic!("return a = b")
    };
    let ExprKind::Call { callee, .. } = db.exprs[e].kind else {
        panic!("a call of =")
    };
    let Some(Body::Implicit(op)) = db.decls[callee].kind.subprogram().map(|s| &s.body) else {
        panic!("predefined =")
    };
    assert_eq!(op.ty, db.well_known.integer.expect("INTEGER"));
    assert_eq!(op.family, crate::implicit::Family::Eq);
    // The generic's own body is untouched.
    let geq = unit_decl(&db, "g", "eq");
    let Some(Body::Hir { stmts, .. }) = db.decls[geq].kind.subprogram().map(|s| &s.body) else {
        panic!("generic body")
    };
    let StmtKind::Return(Some(e)) = db.stmts[stmts[0]].kind else {
        panic!("return")
    };
    let ExprKind::Call { callee, .. } = db.exprs[e].kind else {
        panic!("call")
    };
    let Some(Body::Implicit(op)) = db.decls[callee].kind.subprogram().map(|s| &s.body) else {
        panic!("implicit")
    };
    assert!(matches!(db.types[op.ty].kind, TypeKind::Generic { .. }));
}

#[test]
fn w1506_user_eq_visible_at_the_instantiation() {
    let src = "
package g is
  generic (type t);
  function eq (a, b : t) return boolean;
end package;
library ieee;
use ieee.numeric_std.all;
package i is new work.g generic map (t => unsigned);
";
    let db = analyze(LangRev::Vhdl2008, src);
    assert_eq!(codes(&db), ["W1506"]);
    // Without numeric_std's "=" in sight there is nothing to warn about.
    let db = analyze(
        LangRev::Vhdl2008,
        "package g is generic (type t); subtype s is t; end package;
         library ieee;
         package i is new work.g generic map (t => ieee.numeric_std.unsigned);",
    );
    assert!(codes(&db).is_empty(), "{:?}", codes(&db));
}

#[test]
fn box_default_is_looked_up_at_the_instantiation() {
    let db = clean(
        "
package g is
  generic (type t; function plus (a, b : t) return t is <>);
  function twice (a : t) return t;
end package;
package ops is
  function plus (a, b : bit) return bit;
end package;
use work.ops.all;
package i is new work.g generic map (t => bit);
",
    );
    let i = inst_of(&db, "i");
    let (&f, &a) = db.insts[i]
        .map
        .subprograms
        .iter()
        .find(|(f, _)| db.decls[**f].name.key().as_str() == "plus")
        .expect("plus is mapped");
    assert!(matches!(db.decls[f].kind, DeclKind::GenericSubprogram(_)));
    assert_eq!(
        db.decls[a].scope,
        unit_scope(&db, "ops"),
        "ops.plus is the default"
    );
}

#[test]
fn formal_package_maps_to_the_actual_instance() {
    let db = clean(
        "
package a is
  generic (n : integer := 3);
  type word is array (1 to n) of bit;
end package;
package b is
  generic (package p is new work.a generic map (<>));
  use p.all;
  subtype w2 is word;
end package;
package ai is new work.a generic map (n => 5);
package bi is new work.b generic map (p => work.ai);
",
    );
    let w2 = unit_decl(&db, "bi", "w2");
    let DeclKind::Subtype(st) = db.decls[w2].kind else {
        panic!("w2 is a subtype")
    };
    let word = unit_decl(&db, "ai", "word");
    let DeclKind::Type(t) = db.decls[word].kind else {
        panic!("word is a type")
    };
    assert_eq!(db.base(st), t, "bi.w2 is ai.word, not a.word");
}

#[test]
fn generic_subprogram_instance() {
    let mut db = clean(
        "
package p is
  function id generic (type t) parameter (x : t) return t;
  function idi is new id generic map (t => integer);
end package;
package body p is
  function id generic (type t) parameter (x : t) return t is
  begin
    return x;
  end function;
end package body;
",
    );
    let idi = unit_decl(&db, "p", "idi");
    let s = db.decls[idi].kind.subprogram().expect("idi");
    let int = db.well_known.integer.expect("INTEGER");
    assert_eq!(db.base(s.params[0].subtype), int);
    let Body::Instance(inst) = s.body else {
        panic!("instance body")
    };
    db.instance_body(inst).expect("body");
    let Some(Body::Hir { stmts, .. }) = db.decls[idi].kind.subprogram().map(|s| &s.body) else {
        panic!("cloned body")
    };
    let StmtKind::Return(Some(e)) = db.stmts[stmts[0]].kind else {
        panic!("return x")
    };
    assert_eq!(db.base(db.exprs[e].subtype), int);
    let ExprKind::Obj(x) = db.exprs[e].kind else {
        panic!("x")
    };
    let DeclKind::Object(o) = &db.decls[x].kind else {
        panic!("x is a parameter")
    };
    assert_eq!(db.base(o.subtype), int);
}

#[test]
fn errors_have_their_codes() {
    let e = |src: &str| codes(&analyze(LangRev::Vhdl2008, src));
    // E1501: instantiating a non-generic subprogram.
    assert_eq!(
        e("package p is
             function f (x : integer) return integer;
             function g is new f;
           end package;"),
        ["E1501"]
    );
    // E1502: a formal type without an actual.
    assert_eq!(
        e("package g is generic (type t); subtype s is t; end package;
           package i is new work.g;"),
        ["E1502"]
    );
    // E1505: an uninstantiated package as a prefix.
    assert_eq!(
        e(
            "package g is generic (n : integer := 1); constant c : integer := n; end package;
           use work.g.all;
           package q is end package;"
        ),
        ["E1505"]
    );
    // E1503: a formal package actual of the wrong generic.
    assert_eq!(
        e("package a is generic (n : integer := 1); constant c : integer := n; end package;
           package a2 is generic (n : integer := 1); constant c : integer := n; end package;
           package b is generic (package p is new work.a generic map (<>)); constant c : integer := p.c; end package;
           package x is new work.a2;
           package y is new work.b generic map (p => work.x);"),
        ["E1503"]
    );
    // E1302: two subprograms fit a formal subprogram actual.
    assert_eq!(
        e(
            "package g is generic (function f return integer); end package;
           package s is
             function h return integer;
           end package;
           package s2 is
             function h return integer;
           end package;
           use work.s.all, work.s2.all;
           package i is new work.g generic map (f => h);"
        ),
        ["E1302"]
    );
}

#[test]
fn e1505_call_of_an_uninstantiated_subprogram() {
    let db = analyze(
        LangRev::Vhdl2008,
        "package p is
           function id generic (type t) parameter (x : t) return t;
           constant c : integer := id(1);
         end package;",
    );
    assert_eq!(codes(&db), ["E1505"]);
}

#[test]
fn w1501_unused_generic_after_the_body() {
    let e = |src: &str| codes(&analyze(LangRev::Vhdl2008, src));
    assert_eq!(
        e("package g is generic (n : integer := 1); end package;"),
        ["W1501"]
    );
    // Used only in the body: no warning.
    assert!(
        e("package g is generic (n : integer := 1); function f return integer; end package;
           package body g is function f return integer is begin return n; end function; end package body;")
        .is_empty()
    );
    assert_eq!(
        e("package p is
             function f generic (n : integer := 1) parameter (x : integer) return integer;
           end package;
           package body p is
             function f generic (n : integer := 1) parameter (x : integer) return integer is
             begin return x; end function;
           end package body;"),
        ["W1501"]
    );
}

#[test]
fn package_instance_inside_a_generic_package() {
    let mut db = clean(
        "
package h is
  generic (m : integer := 1);
  constant hm : integer := m * 2;
  function f return integer;
end package;
package body h is
  function f return integer is begin return m; end function;
end package body;
package g is
  generic (n : integer := 1);
  package hi is new work.h generic map (m => n + 1);
  constant k : integer := hi.hm;
end package;
package gi is new work.g generic map (n => 10);
",
    );
    let k = unit_decl(&db, "gi", "k");
    let DeclKind::Object(o) = &db.decls[k].kind else {
        panic!("k")
    };
    assert_eq!(
        db.value(o.default.expect("default")),
        Some(&ConstValue::Int(22))
    );
    // The nested instance's body clones with m => 11.
    let hi = unit_decl(&db, "gi", "hi");
    let DeclKind::PackageInst(i) = db.decls[hi].kind else {
        panic!("hi")
    };
    assert_eq!(db.insts[i].mangled.as_str(), "work.gi.hi");
    db.instance_body(i).expect("body");
    let f = db.scopes[db.insts[i].scope].names[&Symbol::intern("f")][0];
    let Some(Body::Hir { stmts, .. }) = db.decls[f].kind.subprogram().map(|s| &s.body) else {
        panic!("cloned body")
    };
    let StmtKind::Return(Some(e)) = db.stmts[stmts[0]].kind else {
        panic!("return m")
    };
    assert_eq!(db.value(e), Some(&ConstValue::Int(11)));
}

#[test]
fn subprogram_instance_inside_a_generic_package() {
    let mut db = clean(
        "
package p is
  function id generic (type t) parameter (x : t) return t;
end package;
package body p is
  function id generic (type t) parameter (x : t) return t is begin return x; end function;
end package body;
package g is
  generic (type u);
  function idu is new work.p.id generic map (t => u);
end package;
package gi is new work.g generic map (u => bit);
",
    );
    let f = unit_decl(&db, "gi", "idu");
    let s = db.decls[f].kind.subprogram().expect("idu");
    assert_eq!(
        db.base(s.params[0].subtype),
        db.well_known.bit.expect("BIT")
    );
    let Body::Instance(i) = s.body else {
        panic!("instance")
    };
    let gf = unit_decl(&db, "g", "idu");
    let Some(&Body::Instance(gi)) = db.decls[gf].kind.subprogram().map(|s| &s.body) else {
        panic!("instance in g")
    };
    assert_ne!(i, gi, "the clone has its own instance");
    db.instance_body(i).expect("body");
    let Some(Body::Hir { stmts, .. }) = db.decls[f].kind.subprogram().map(|s| &s.body) else {
        panic!("cloned body")
    };
    let StmtKind::Return(Some(e)) = db.stmts[stmts[0]].kind else {
        panic!("return x")
    };
    assert_eq!(
        db.base(db.exprs[e].subtype),
        db.well_known.bit.expect("BIT")
    );
}

#[test]
fn formal_package_with_a_generic_map() {
    let pre = "
package a is
  generic (n : integer := 1; type t);
  constant c : integer := n;
  subtype s is t;
end package;
package b is
  generic (package p is new work.a generic map (n => 3, t => bit));
  constant k : integer := p.c;
end package;
";
    let e = |rest: &str| codes(&analyze(LangRev::Vhdl2008, &format!("{pre}{rest}")));
    assert!(
        e("package x is new work.a generic map (n => 3, t => bit);
           package y is new work.b generic map (p => work.x);")
        .is_empty()
    );
    assert_eq!(
        e("package x is new work.a generic map (n => 4, t => bit);
           package y is new work.b generic map (p => work.x);"),
        ["E1503"]
    );
    assert_eq!(
        e("package x is new work.a generic map (n => 3, t => integer);
           package y is new work.b generic map (p => work.x);"),
        ["E1503"]
    );
}

#[test]
fn generic_protected_type() {
    let pkg = "
package p is
  type pt is protected
    generic (type t; n : integer := 1);
    procedure set (x : t);
  end protected;
end package;
package body p is
  type pt is protected body
    variable v : t;
    variable k : integer := n;
    procedure set (x : t) is begin v := x; end procedure;
  end protected body;
end package body;
";
    let db = analyze(LangRev::Vhdl2019, pkg);
    assert!(codes(&db).is_empty(), "{:?}", codes(&db));
    let db = analyze(
        LangRev::Vhdl2019,
        &format!("{pkg}package q is shared variable sv : work.p.pt; end package;"),
    );
    assert_eq!(codes(&db), ["E1505"]);
}
