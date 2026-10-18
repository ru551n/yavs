//! VHDL-2019 reflection (sem.md 2.11): the HIR M3 builds mirrors from.

use std::path::PathBuf;

use yavs_diag::{MessageFormat, render};
use yavs_sem::hir::{AttrPrefix, ExprKind, PredefAttr, Staticness};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

const SRC: &str = "
use std.reflection.all;
package p is
  type rec is record
    a : integer;
  end record;
  impure function f(x : rec) return value_class;
end package;
package body p is
  impure function f(x : rec) return value_class is
    variable m : value_mirror := x'reflect;
    variable s : subtype_mirror := rec'reflect;
  begin
    return m.get_value_class;
  end function;
end package body;
";

/// Each test writes its own file: the tests run in parallel.
fn analyze(test: &str) -> Db {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("reflect")
        .join(test);
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("p.vhd");
    std::fs::write(&path, SRC).expect("write source");
    let mut db = Db::new(Options::new(LangRev::Vhdl2019, default_stdlib()));
    let r = db.analyze_file("work", &path, LangRev::Vhdl2019);
    let text: String = db
        .diagnostics()
        .iter()
        .map(|d| render(&db.sources, d, MessageFormat::Human, false))
        .collect();
    assert!(r.is_ok() && db.diagnostics().is_empty(), "{text}");
    db
}

fn type_name(db: &Db, st: yavs_sem::SubtypeId) -> String {
    let t = db.subtypes[st].base;
    db.decls[db.types[t].decl].name.to_string()
}

#[test]
fn reflect_is_a_value_or_subtype_mirror_and_not_static() {
    let db = analyze("attr");
    let mut found = Vec::new();
    for (_, e) in db.exprs.iter() {
        if let ExprKind::Attr {
            attr: PredefAttr::Reflect,
            prefix,
            ..
        } = e.kind
        {
            assert_eq!(e.stat, Staticness::None);
            let what = match prefix {
                AttrPrefix::Expr(p) => format!("value of {}", type_name(&db, db.exprs[p].subtype)),
                AttrPrefix::Subtype(st) => format!("subtype {}", type_name(&db, st)),
                AttrPrefix::Entity(_) => "entity".into(),
            };
            found.push((what, type_name(&db, e.subtype)));
        }
    }
    found.sort();
    assert_eq!(
        found,
        [
            ("subtype rec".to_string(), "SUBTYPE_MIRROR".to_string()),
            ("value of rec".to_string(), "VALUE_MIRROR".to_string()),
        ]
    );
}

#[test]
fn mirror_methods_are_calls_on_the_dereferenced_access_value() {
    let db = analyze("method");
    let calls: Vec<_> = db
        .exprs
        .iter()
        .filter_map(|(_, e)| match e.kind {
            ExprKind::Call {
                callee,
                method_of: Some(o),
                ..
            } => Some((callee, o)),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    let (callee, o) = calls[0];
    assert_eq!(db.decls[callee].name.to_string(), "get_value_class");
    let ExprKind::Deref { prefix } = db.exprs[o].kind else {
        panic!("the method object is not dereferenced");
    };
    assert!(matches!(db.exprs[prefix].kind, ExprKind::Obj(_)));
    assert_eq!(type_name(&db, db.exprs[o].subtype), "VALUE_MIRROR_PT");
}

/// VUnit record_dict_pkg's shape: a generic function reflects its parameter.
const GENERIC: &str = "
use std.reflection.all;
package g is
  impure function class_of generic (type t) parameter (value : t) return value_class;
end package;
package body g is
  impure function class_of generic (type t) parameter (value : t) return value_class is
    variable m : value_mirror := value'reflect;
  begin
    return m.get_value_class;
  end function;
end package body;
use std.reflection.all;
use work.g.all;
package u is
  type rec is record
    a : integer;
  end record;
  function class_of is new class_of generic map (t => rec);
  function class_of is new class_of generic map (t => integer);
  constant c : value_class := class_of(rec'(a => 1));
  constant d : value_class := class_of(3);
end package;
";

#[test]
fn a_generic_subprogram_instance_reflects_its_actual_type() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("reflect")
        .join("generic");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("g.vhd");
    std::fs::write(&path, GENERIC).expect("write source");
    let mut db = Db::new(Options::new(LangRev::Vhdl2019, default_stdlib()));
    let r = db.analyze_file("work", &path, LangRev::Vhdl2019);
    let insts: Vec<_> = db.insts.iter().map(|(i, _)| i).collect();
    for i in insts {
        assert!(db.instance_body(i).is_ok());
    }
    let text: String = db
        .diagnostics()
        .iter()
        .map(|d| render(&db.sources, d, MessageFormat::Human, false))
        .collect();
    assert!(r.is_ok() && db.diagnostics().is_empty(), "{text}");
    let mut reflected: Vec<String> = db
        .exprs
        .iter()
        .filter_map(|(_, e)| match e.kind {
            ExprKind::Attr {
                attr: PredefAttr::Reflect,
                prefix: AttrPrefix::Expr(p),
                ..
            } => Some(type_name(&db, db.exprs[p].subtype)),
            _ => None,
        })
        .collect();
    reflected.sort();
    assert_eq!(reflected, ["INTEGER", "rec", "t"]);
}
