//! Compiled code stays found: an unchanged program makes the same bodies, whatever the policy
//! compiles with a function in this run (#1035). One test: the store is one per process.
#![cfg(feature = "clift")]

use std::path::Path;
use std::sync::Mutex;

use yavs_clift::First;
use yavs_diag::{FileId, Span};
use yavs_ir::*;
use yavs_rt::Generator;
use yavs_rt::native::Part;

/// `inc(a)`, a chain of additions ending in `step`, too long for the optimiser to inline, and
/// `caller(a) = inc(a)`.
fn program(step: i64) -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let inc = pb.declare("inc", FuncKind::Function, None);
    let caller = pb.declare("caller", FuncKind::Function, None);
    let arg = |fb: &mut FuncBuilder<'_>| {
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let p = fb.addr(AddrSpace::Frame, s);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst,
            addr: p,
            w: Width::B8,
        });
        dst
    };
    {
        let mut fb = pb.define(inc);
        let a = arg(&mut fb);
        let mut acc = fb.int(step);
        for _ in 0..80 {
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Add,
                dst,
                a,
                b: acc,
            });
            acc = dst;
        }
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
    }
    {
        let mut fb = pb.define(caller);
        let a = arg(&mut fb);
        let args = fb.regs(&[a]);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: inc,
            args,
            dst: Some(dst),
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
    }
    (pb.finish().expect("verified"), caller, inc)
}

/// A run's compile of `caller` with `tree`, and the run's end (its routes are stored then).
fn compile(prog: &Program, caller: FuncId, tree: &[FuncId]) {
    let mut g = First::new().expect("a code generator");
    g.tree(tree);
    let f = prog.func(caller).expect("the function");
    g.compile(prog, caller, f, Part::Native).expect("compiled");
    g.cache_flush();
}

fn compiled(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("yavs-cache").join("obj")).map_or(0, |d| {
        d.filter(|e| {
            e.as_ref()
                .is_ok_and(|e| e.file_name().to_string_lossy().starts_with("clif-"))
        })
        .count()
    })
}

static ONE: Mutex<()> = Mutex::new(());

#[test]
fn a_body_does_not_follow_what_compiles_with_it() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("yavs_stable_keys_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("scratch");
    yavs_clift::set_cache(Some(&tmp));

    let (prog, caller, inc) = program(1);
    // First run: the callee compiles with the caller, so the caller calls it directly.
    compile(&prog, caller, &[caller, inc]);
    let n = compiled(&tmp);
    assert!(n >= 2, "caller and callee stored: {n}");
    let (hits, stored) = yavs_clift::cache_stats();

    // Later runs: the samples may leave the callee out. Nothing new, all from the store.
    for tree in [&[caller][..], &[caller, inc][..]] {
        compile(&prog, caller, tree);
        assert_eq!(compiled(&tmp), n, "tree {tree:?} made another body");
    }
    let (hits2, stored2) = yavs_clift::cache_stats();
    assert_eq!(stored2, stored, "nothing compiled again");
    assert!(hits2 > hits, "linked from the store");

    // Another design misses.
    let (other, caller2, inc2) = program(2);
    compile(&other, caller2, &[caller2, inc2]);
    assert!(compiled(&tmp) > n, "another callee is another entry");

    yavs_clift::set_cache(None);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The entries of the two lists under `dir` (the compiled functions, the routes), with what tells a
/// rewrite: size, time and, where there is one, the file's identity.
fn lists(dir: &Path) -> Vec<(String, u64, std::time::SystemTime, u64)> {
    let mut v: Vec<_> = std::fs::read_dir(dir.join("yavs-cache").join("obj"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("list-") || name.starts_with("route-")) {
                return None;
            }
            let m = e.metadata().ok()?;
            #[cfg(unix)]
            let ino = std::os::unix::fs::MetadataExt::ino(&m);
            #[cfg(not(unix))]
            let ino = 0;
            Some((name, m.len(), m.modified().ok()?, ino))
        })
        .collect();
    v.sort();
    v
}

/// The end of a run that cannot wait for a compile in flight (`AutoTier::stop`, a signal) stores
/// the lists from its own thread: what the compiles noted is on disk after one call, a second call
/// with nothing new writes nothing (no file operation), and new lines are stored by the next.
#[test]
fn the_lists_are_stored_from_another_thread_once() {
    let _one = ONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("yavs_stop_lists_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("scratch");
    yavs_clift::set_cache(Some(&tmp));

    let (prog, caller, inc) = program(1);
    let mut g = First::new().expect("a code generator");
    g.tree(&[caller, inc]);
    let f = prog.func(caller).expect("the function");
    g.compile(&prog, caller, f, Part::Native).expect("compiled");
    g.cache_note("caller", 7);
    assert!(
        lists(&tmp).is_empty(),
        "nothing is stored before it is asked"
    );

    let flush = g
        .cache_flusher()
        .expect("a store a thread of the run's end can call");
    std::thread::spawn(flush).join().expect("flush");
    let first = lists(&tmp);
    let names: Vec<&str> = first.iter().map(|l| l.0.as_str()).collect();
    assert!(
        names.iter().any(|n| n.starts_with("list-"))
            && names.iter().any(|n| n.starts_with("route-")),
        "both lists: {names:?}"
    );

    // Nothing new: no write, from either side.
    flush();
    g.cache_flush();
    assert_eq!(lists(&tmp), first, "a second store with nothing new wrote");

    // A compile after it: the next store has it.
    g.cache_note("later", 9);
    std::thread::spawn(flush).join().expect("flush");
    assert_ne!(lists(&tmp), first, "the new line was not stored");

    yavs_clift::set_cache(None);
    let _ = std::fs::remove_dir_all(&tmp);
}
