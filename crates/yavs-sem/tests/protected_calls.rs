//! Method calls of protected types (4.5.4): through an object they carry the object as
//! `method_of`; inside the protected body they use the implicit object (`method_of: None`).

use yavs_sem::hir::{ExprKind, StmtKind};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

#[test]
fn method_calls_carry_their_object() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/diag/sem/prot/method_calls_ok.vhd");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    let r = db.analyze_file("work", &path, LangRev::Vhdl2008);
    assert!(r.is_ok() && db.diagnostics().is_empty());
    let name = |db: &Db, d| db.decls[d].name.key().as_str().to_string();
    let mut procs: Vec<(String, bool)> = db
        .stmts
        .iter()
        .filter_map(|(_, s)| match &s.kind {
            StmtKind::ProcCall(c) => Some((name(&db, c.callee), c.method_of.is_some())),
            _ => None,
        })
        .collect();
    procs.sort();
    // `inc;` inside the body has no object; c.inc (3 calls), sc.inc, work.pk.sc.inc do.
    let inc_without: usize = procs.iter().filter(|(n, m)| n == "inc" && !m).count();
    let inc_with: usize = procs.iter().filter(|(n, m)| n == "inc" && *m).count();
    assert_eq!((inc_without, inc_with), (1, 5), "{procs:?}");
    // Function methods in expressions: c.get, c.get_plus(1), c.name(1), work.pk.sc.get,
    // c.name; `get` inside get_plus has no object.
    let fn_calls: Vec<(String, bool)> = db
        .exprs
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            ExprKind::Call {
                callee, method_of, ..
            } if matches!(name(&db, *callee).as_str(), "get" | "get_plus" | "name") => {
                Some((name(&db, *callee), method_of.is_some()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(fn_calls.iter().filter(|c| c.1).count(), 5, "{fn_calls:?}");
    assert!(
        fn_calls.contains(&("get".to_string(), false)),
        "{fn_calls:?}"
    );
}
