//! Without `YAVS_CLIFT_BLOCKCOV` the tier keeps no table and the dump writes nothing.
#![cfg(feature = "clift")]

use yavs_diag::{FileId, Span};
use yavs_ir::*;

#[test]
fn the_switch_off_lists_no_body() {
    assert!(std::env::var_os("YAVS_CLIFT_BLOCKCOV").is_none());
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("off", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let v = fb.int(7);
    fb.emit(Inst::Ret { v: Some(v) });
    fb.finish();
    let prog = pb.finish().unwrap();
    yavs_clift::compile(&prog, usize::MAX).unwrap();
    assert!(yavs_clift::blockcov_snapshot().is_empty());
    yavs_clift::blockcov_write(None);
}
