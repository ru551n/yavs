//! Positive corpus: the vendored `lib/` tree analyzes with zero diagnostics
//! (sem.md 9.1). Extend `UNITS` as slices land (sem.md 10, "green when").

use rsim_diag::{MessageFormat, render};
use rsim_sem::stat::ConstValue;
use rsim_sem::{Db, LangRev, Options, default_stdlib};

/// `(library, primary unit)` loaded on demand, per revision.
const UNITS: &[(&str, &str)] = &[("std", "standard")];

fn analyze(rev: LangRev, lib: &str, unit: &str) -> Db {
    let mut db = Db::new(Options::new(rev, default_stdlib()));
    let r = db.load_primary(lib, unit);
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

#[test]
fn stdlib_units_analyze_clean() {
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        for (lib, unit) in UNITS {
            analyze(rev, lib, unit);
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
