//! `YAVS_LLVM_KINDLOC` (the measuring aid) on a whole-program module of several functions (#773).
//! Its own test binary: the variable is process-wide.
#![cfg(feature = "llvm")]
#![allow(unsafe_code)] // (`set_var`, before any other thread exists in this binary)

use yavs_diag::{FileId, Span};
use yavs_ir::*;
use yavs_llvm::OptLevel;

#[test]
fn kindloc_compiles_a_module_of_several_functions() {
    let mut pb = ProgramBuilder::new();
    for name in ["f", "g", "h"] {
        let f = pb.declare(name, FuncKind::Procedure, None);
        let mut fb = pb.define(f);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let prog = pb.finish().unwrap();
    let out = std::env::temp_dir().join(format!("kindloc_{}", std::process::id()));
    // SAFETY: the only test of this binary, so no other thread reads the environment.
    unsafe { std::env::set_var("YAVS_LLVM_KINDLOC", &out) };
    let tier = yavs_llvm::compile(&prog, OptLevel::O2, usize::MAX);
    let _ = std::fs::remove_file(&out);
    let tier = tier.expect("the module verifies");
    assert!(tier.notes.is_empty(), "{:?}", tier.notes);
}
