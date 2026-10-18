//! Positive corpus: the vendored `lib/` tree analyzes with zero diagnostics
//! (sem.md 9.1). Extend `UNITS` as slices land (sem.md 10, "green when").

use rsim_diag::{MessageFormat, render};
use rsim_sem::stat::ConstValue;
use rsim_sem::{Db, LangRev, Options, default_stdlib};

/// `(library, primary unit)` loaded on demand, per revision.
const UNITS: &[(&str, &str)] = &[
    ("std", "standard"),
    ("std", "textio"),
    ("std", "env"),
    ("ieee", "std_logic_1164"),
    ("ieee", "std_logic_textio"),
    ("ieee", "numeric_std"),
    ("ieee", "numeric_std_unsigned"),
    ("ieee", "numeric_bit"),
    ("ieee", "numeric_bit_unsigned"),
    ("ieee", "math_real"),
    ("ieee", "math_complex"),
    ("ieee", "fixed_float_types"),
    ("ieee", "fixed_generic_pkg"),
    ("ieee", "fixed_pkg"),
    ("ieee", "float_generic_pkg"),
    ("ieee", "float_pkg"),
];

/// Packages whose bodies analyze clean too.
const BODIES: &[&str] = &[
    "math_real",
    "math_complex",
    "numeric_std_unsigned",
    "numeric_bit_unsigned",
];

fn analyze(rev: LangRev, lib: &str, unit: &str) -> Db {
    analyze_with(rev, lib, unit, false)
}

fn analyze_with(rev: LangRev, lib: &str, unit: &str, body: bool) -> Db {
    let mut db = Db::new(Options::new(rev, default_stdlib()));
    let mut r = db.load_primary(lib, unit).map(|_| ());
    if body && r.is_ok() {
        r = db.load_body(lib, unit).map(|_| ());
    }
    let text: String = db
        .diagnostics()
        .iter()
        .map(|d| render(&db.sources, d, MessageFormat::Human, false))
        .collect();
    assert!(
        r.is_ok() && db.diagnostics().is_empty(),
        "{lib}.{unit} ({rev:?}):\n{text}"
    );
    db
}

/// `std.reflection` exists only in 2019; its methods return incomplete types (U-2).
#[test]
fn reflection_analyzes_clean() {
    analyze(LangRev::Vhdl2019, "std", "reflection");
}

#[test]
fn stdlib_units_analyze_clean() {
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        for (lib, unit) in UNITS {
            analyze(rev, lib, unit);
        }
    }
}

#[test]
fn ieee_bodies_analyze_clean() {
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        for unit in BODIES {
            analyze_with(rev, "ieee", unit, true);
        }
    }
}

#[test]
fn integer_range_follows_the_revision() {
    for (rev, high) in [
        (LangRev::Vhdl2008, i64::from(i32::MAX)),
        (LangRev::Vhdl2019, i64::MAX),
    ] {
        let db = analyze(rev, "std", "standard");
        let int = db.well_known.integer.expect("INTEGER");
        let (_, _, r) = db
            .static_bounds(db.types[int].first_subtype)
            .expect("static range");
        assert_eq!(r, ConstValue::Int(high));
        let nat = db.well_known.natural.expect("NATURAL");
        assert_eq!(
            db.static_bounds(nat).map(|b| b.2),
            Some(ConstValue::Int(high))
        );
        let time = db.well_known.time.expect("TIME");
        assert!(db.static_bounds(db.types[time].first_subtype).is_some());
    }
}

/// Rule 1 checkpoint (sem.md 10, S3): numeric_std's array subtypes in the one
/// subtype representation. `UNSIGNED` is `(resolved) UNRESOLVED_UNSIGNED`: no
/// constraint of its own, an element resolution naming `resolved`, and the base
/// is an array indexed by NATURAL with STD_ULOGIC elements.
#[test]
fn numeric_std_subtypes_rule_1() {
    use rsim_sem::decl::DeclKind;
    use rsim_sem::types::{Constrainedness, Resolution, TypeKind};
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    let u = db.load_primary("ieee", "numeric_std").expect("numeric_std");
    let scope = db.units[u].scope;
    let find = |db: &Db, key: &str| {
        let ds = &db.scopes[scope].names[&rsim_syntax::Symbol::intern(key)];
        match db.decls[ds[0]].kind {
            DeclKind::Subtype(st) => st,
            DeclKind::Type(t) => db.types[t].first_subtype,
            _ => panic!("{key} is not a type"),
        }
    };
    for (name, unres) in [
        ("unsigned", "unresolved_unsigned"),
        ("signed", "unresolved_signed"),
    ] {
        let st = find(&db, name);
        let base = db.base(st);
        assert_eq!(
            base,
            db.base(find(&db, unres)),
            "{name} has the base of {unres}"
        );
        assert_eq!(db.constrainedness(st), Constrainedness::Unconstrained);
        let Some(Resolution::Element(inner)) = &db.subtypes[st].resolution else {
            panic!("{name}: element resolution expected");
        };
        let Resolution::Function(f) = **inner else {
            panic!("{name}: resolution function expected");
        };
        assert_eq!(db.decls[f].name.key().as_str(), "resolved");
        let TypeKind::Array { indexes, elem } = &db.types[base].kind else {
            panic!("{name}: array base expected");
        };
        assert_eq!(indexes.len(), 1);
        assert_eq!(Some(indexes[0]), db.well_known.natural);
        assert_eq!(
            db.decls[db.types[db.base(*elem)].decl].name.key().as_str(),
            "std_ulogic"
        );
        assert!(db.subtypes[find(&db, unres)].resolution.is_none());
    }
}
