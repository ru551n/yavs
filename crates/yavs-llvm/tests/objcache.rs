//! Compiled code kept across compiles (#505): one test, because the cache is set per process.
#![cfg(feature = "llvm")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime};
use yavs_ir::*;
use yavs_llvm::{AutoTier, Heat, Note, OptLevel, cache_stats, set_cache};

/// `function sum(n) return integer`: 0 + 1 + ... + n in a loop.
fn sum() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("sum", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: n,
        addr: pn,
        w: Width::B8,
    });
    let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Le,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    for (dst, b) in [(acc, i), (i, one)] {
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst,
            a: dst,
            b,
        });
    }
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

fn objects(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir.join("yavs-cache/obj"))
        .map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "img"))
        .collect()
}

/// Runs `f` of a fresh runtime of `prog` with `tier` until it has run compiled code.
fn run_eager(prog: &Arc<Program>, f: FuncId) -> Option<u64> {
    let tier = yavs_llvm::compile(prog, OptLevel::O2, usize::MAX).unwrap();
    assert!(tier.notes.is_empty(), "{:?}", tier.notes);
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    assert!(rt.set_tier(Arc::new(tier)));
    yavs_interp::call(rt, f, Arc::new(InstData::default()), &[100_000]).unwrap()
}

/// `--tier=auto` with the payback rule: the loop is compiled (cold) or linked from the cache by
/// the probe at the plain threshold (warm); returns once it runs compiled.
fn run_auto(prog: &Arc<Program>, f: FuncId) {
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let heat = Heat {
        base: 1,
        per_instr: 0,
        period: 100,
        budget: 2,
    };
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, heat, note, None).unwrap();
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
        assert_eq!(r.unwrap(), Some(5_000_050_000));
    }
    tier.stop(true);
}

#[test]
fn compiled_code_is_kept_and_linked_again() {
    let dir = tempfile::tempdir().unwrap();
    let (prog, f) = sum();
    let prog = Arc::new(prog);
    let want = Some(5_000_050_000);

    // Without a cache nothing is stored.
    assert_eq!(run_eager(&prog, f), want);
    assert_eq!(cache_stats(), (0, 0));

    set_cache(Some(dir.path()));
    assert_eq!(run_eager(&prog, f), want);
    assert_eq!(cache_stats(), (0, 1), "cold: compiled and stored");
    assert_eq!(objects(dir.path()).len(), 1);
    assert_eq!(run_eager(&prog, f), want);
    assert_eq!(cache_stats(), (1, 1), "warm: linked");

    // A damaged entry is a miss: compiled and stored again.
    for o in objects(dir.path()) {
        std::fs::write(o, b"garbage").unwrap();
    }
    assert_eq!(run_eager(&prog, f), want);
    assert_eq!(cache_stats(), (1, 2));
    assert_eq!(run_eager(&prog, f), want);
    assert_eq!(cache_stats(), (2, 2));

    // The tiered mode: modules of its own for the function's native-frame and frame-mode code,
    // linked by the probe on the warm run.
    run_auto(&prog, f);
    let (hits, stored) = cache_stats();
    assert_eq!(stored, 4, "cold: compiled and stored");
    run_auto(&prog, f);
    assert_eq!(cache_stats(), (hits + 2, stored), "warm: linked");

    // A directory that cannot be written: compiled, nothing kept, no error.
    let file = dir.path().join("plain-file");
    std::fs::write(&file, b"").unwrap();
    set_cache(Some(&file));
    assert_eq!(run_eager(&prog, f), want);
    assert!(!file.join("yavs-cache").exists());
}
