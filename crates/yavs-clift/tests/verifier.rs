//! A Cranelift verifier error is a bug of the lowering and must be loud in debug builds: the
//! eager tier stops, and the compile thread of `--tier=auto` re-raises its panic when the tier
//! stops (a function silently left interpreted would hide it).
#![cfg(all(feature = "clift", debug_assertions))]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use yavs_clift::First;
use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime};
use yavs_ir::*;
use yavs_llvm::{AutoTier, Gens, Heat, Note, OptLevel};

/// `function f return 7`.
fn seven() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let v = fb.int(7);
    fb.emit(Inst::Ret { v: Some(v) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn verifier_errors_are_not_swallowed() {
    // (The flag is process-wide: one test in this binary.)
    yavs_clift::BREAK.store(true, std::sync::atomic::Ordering::Relaxed);
    let (prog, f) = seven();
    let eager = catch_unwind(AssertUnwindSafe(|| yavs_clift::compile(&prog, usize::MAX)));
    let msg = eager.err().expect("the eager tier panics");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("Cranelift rejected"), "{msg}");

    // Eager policy: the function is compiled as the interpreter enters it, on the tier-up thread.
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let heat = Heat {
        base: 0,
        per_instr: 0,
        period: 0,
        budget: 0,
    };
    let note: Note = Box::new(|_, _, _| {});
    let gens = Gens {
        main: None,
        first: Some(Box::new(First::new().unwrap())),
    };
    let tier = AutoTier::start_with(&rt, OptLevel::O2, usize::MAX, heat, note, gens).unwrap();
    let _ = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let stopped = catch_unwind(AssertUnwindSafe(|| tier.stop(true)));
    assert!(stopped.is_err(), "the compile thread's panic is re-raised");
}
