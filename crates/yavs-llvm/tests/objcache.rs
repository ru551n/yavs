//! Compiled code kept across compiles (#505): one test, because the cache is set per process.
#![cfg(feature = "llvm")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime, Tier};
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

/// A process that reads generic `G` (instance offset 0) and stores 1 or 2 in a frame slot
/// depending on `G == 3`, with room for two copies.
fn process_with_copies() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    let (head, body, yes, no, end) = (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    fb.body(head);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.emit(Inst::Br { to: body });
    fb.at(body);
    let a = fb.addr(AddrSpace::Inst, 0);
    let g = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: g,
        addr: a,
        w: Width::B8,
    });
    let three = fb.int(3);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: g,
        b: three,
    });
    fb.emit(Inst::CondBr { c, t: yes, f: no });
    for (blk, v) in [(yes, 1), (no, 2)] {
        fb.at(blk);
        let s = fb.addr(AddrSpace::Frame, 0);
        let k = fb.int(v);
        fb.emit(Inst::Store {
            addr: s,
            src: k,
            w: Width::B8,
        });
        fb.emit(Inst::Br { to: end });
    }
    fb.at(end);
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    let mut p = pb.finish().unwrap();
    p.reserve_copies(2);
    (p, f)
}

/// Compiles the copies of `process_with_copies` for `G` = `order` (in that order, so the first
/// gets the first reserved function id) with the eager tier.
fn compile_copies(order: [i64; 2]) {
    let (prog, f) = process_with_copies();
    let ids: Vec<FuncId> = order
        .iter()
        .map(|&g| {
            let mut d = g.to_le_bytes().to_vec();
            d.extend([0; 8]);
            prog.copy_of(f, &d, &[]).expect("a copy")
        })
        .collect();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let heat = Heat {
        base: 0,
        per_instr: 0,
        period: 0,
        budget: 0,
    };
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, heat, note, None).unwrap();
    for id in ids {
        tier.hot(id);
    }
    assert_eq!(tier.compiled(), 2);
    tier.stop(true);
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

    // The copies of a process are named and keyed by what they are for, not by their function
    // ids, which follow the order the processes switched in: the same copies asked for in the
    // other order are linked from the cache (#661).
    if yavs_ir::opt::specialise_on() {
        let (_, before) = cache_stats();
        compile_copies([3, 4]);
        let (hits, stored) = cache_stats();
        assert!(stored > before, "cold: compiled and stored");
        compile_copies([4, 3]);
        let (more, again) = cache_stats();
        assert_eq!(again, stored, "the other order compiles nothing");
        assert!(more >= hits + 2, "and links both copies");

        // Copies whose modules are the same up to their names (here G = 4, 5 and 6: data the code
        // does not read) are one module: the cache has it from G = 4, found under the name its
        // content gives, and the second copy of the run is that code without asking the cache.
        compile_copies([5, 6]);
        let (twin_hits, twin_stored) = cache_stats();
        assert_eq!(twin_stored, again, "nothing new to compile");
        assert_eq!(
            twin_hits,
            more + 2,
            "one linked from the cache, one the same code"
        );
    }

    // A directory that cannot be written: compiled, nothing kept, no error.
    let file = dir.path().join("plain-file");
    std::fs::write(&file, b"").unwrap();
    set_cache(Some(&file));
    assert_eq!(run_eager(&prog, f), want);
    assert!(!file.join("yavs-cache").exists());
}
