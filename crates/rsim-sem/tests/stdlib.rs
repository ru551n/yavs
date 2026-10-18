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
    "numeric_std",
    "numeric_bit",
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

/// Rule 1 review (sem.md 10, S3) on the numeric_std body: every object's
/// bounds come from one subtype representation. Static bounds are folded
/// (`Expr::value`), the others stay expressions, and nothing else decides.
#[test]
fn numeric_std_body_rule_1() {
    use rsim_sem::decl::{DeclKind, ObjRole};
    use rsim_sem::stat::Staticness;
    use rsim_sem::types::{BoundsFrom, Constraint, RangeC};
    let db = analyze_with(LangRev::Vhdl2008, "ieee", "numeric_std", true);
    let mut tally = std::collections::BTreeMap::<String, usize>::new();
    for (_, d) in db.decls.iter() {
        if !db
            .sources
            .get(d.span.file)
            .path
            .ends_with("numeric_std-body.vhdl")
        {
            continue;
        }
        let DeclKind::Object(o) = &d.kind else {
            continue;
        };
        let shape = match &db.subtypes[o.subtype].constraint {
            Constraint::Array {
                dims: Some(dims), ..
            } => {
                let mut kinds = Vec::new();
                for r in dims.iter() {
                    kinds.push(match r {
                        RangeC::Explicit { left, right, .. } => {
                            for b in [*left, *right] {
                                let e = &db.exprs[b];
                                // Folded exactly when locally static (rule 1: the value is a cache).
                                assert_eq!(
                                    e.value.is_some(),
                                    e.stat == Staticness::Locally,
                                    "{}",
                                    d.name
                                );
                            }
                            if db.exprs[*left].value.is_some() && db.exprs[*right].value.is_some() {
                                "static"
                            } else {
                                "expr"
                            }
                        }
                        RangeC::Attr { .. } => "attr",
                    });
                }
                format!("array({})", kinds.join(","))
            }
            Constraint::Array { dims: None, .. } => "array(open)".into(),
            Constraint::None => "none".into(),
            Constraint::Range(_) => "range".into(),
            Constraint::Record { .. } => "record".into(),
            Constraint::FromObject { .. } => "from-object".into(),
        };
        // An object whose subtype is not fully constrained takes its bounds from
        // its initial value or its actual, never from a guess.
        let full = db.constrainedness(o.subtype) == rsim_sem::types::Constrainedness::Full;
        match (full, o.bounds, &o.role) {
            (true, BoundsFrom::Subtype, _) | (false, BoundsFrom::Initializer, _) => {}
            (false, BoundsFrom::Actual, ObjRole::Param) => {}
            other => panic!("{}: {:?} bounds {:?}", d.name, other.0, other.1),
        }
        *tally
            .entry(format!("{shape} from {:?}", o.bounds))
            .or_default() += 1;
    }
    println!("{tally:#?}");
    assert!(
        tally.keys().any(|k| k.starts_with("array(expr)")),
        "dynamic bounds such as UNSIGNED(L'length-1 downto 0)"
    );
}
