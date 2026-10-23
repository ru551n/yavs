//! Package objects at run time (#183): a package's constants are evaluated once by its
//! initialisation thunk, shared variables are created there, and functions read the package
//! data afterwards.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use yavs_interp::{Bindings, Runtime, SharedState};
use yavs_lower::{LowerOptions, Lowerer};
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, LangRev, Options};

#[test]
fn constants_evaluated_once() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg/pb.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    db.load_body("ieee", "std_logic_1164").unwrap();
    assert_eq!(db.error_count(), 0);
    let pkg = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::Package { .. }) && u.name.key.as_str() == "pb")
        .map(|(id, _)| id)
        .unwrap();
    let func = |name: &str| {
        db.decls
            .iter()
            .find(|(_, d)| {
                d.name.to_string().eq_ignore_ascii_case(name) && d.kind.subprogram().is_some()
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let (get_c1, new_id, get_v) = (func("get_c1"), func("new_id"), func("get_v"));
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let p = l.package(pkg).unwrap();
    let fs = [
        l.subprogram(get_c1).unwrap(),
        l.subprogram(new_id).unwrap(),
        l.subprogram(get_v).unwrap(),
    ];
    let lowered = l.finish().unwrap();
    let pir = lowered.packages[p].clone();
    assert_eq!(pir.signals.len(), 1);
    let prog = Arc::new(lowered.program);
    let shared = Arc::new(Mutex::new(SharedState::new(Vec::new())));
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    let data = yavs_interp::init_package(rt, pir.init, pir.index, pir.size as usize, Some(&shared))
        .unwrap();
    let b = Bindings {
        packages: vec![Arc::new(data)],
        ..Default::default()
    };
    let rt = Runtime::new(prog, b).unwrap();
    let call = |f| {
        yavs_interp::call_shared(rt.clone(), f, Arc::default(), &[], Some(&shared))
            .unwrap()
            .unwrap()
    };
    // c1 took the first id at initialisation; later calls see the same value.
    assert_eq!(call(fs[0]), 1);
    assert_eq!(call(fs[0]), 1);
    assert_eq!(call(fs[1]), 2);
    // A composite constant in package data, returned as a fat pair.
    assert_ne!(call(fs[2]), 0);
}

/// The name of an item of a library-unit package is the same in every design, so a subprogram that
/// reads it (and its callers) can be lowered ahead of the design (`Lowerer::context_dependent`, #460).
#[test]
fn subprograms_that_read_a_library_package_name_do_not_depend_on_the_design() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg/ctx.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let func = |name: &str| {
        db.decls
            .iter()
            .find(|(_, d)| {
                d.name.to_string().eq_ignore_ascii_case(name)
                    && d.kind
                        .subprogram()
                        .is_some_and(|s| matches!(s.body, yavs_sem::decl::Body::Hir { .. }))
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let (who, wrap, plain) = (func("who"), func("wrap"), func("plain"));
    let mut l = Lowerer::new(&db, LowerOptions::default());
    for d in [plain, wrap, who] {
        l.subprogram(d).unwrap();
    }
    assert!(l.context_dependent().is_empty());
    assert_eq!(l.context_users(), 0);
}
