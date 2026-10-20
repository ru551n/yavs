//! With a debugger attached (`opt::keep_variables`) the variables of subprograms stay in
//! their frame slots. A process of its own: the setting is global.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

#[test]
fn a_named_local_keeps_its_slot_for_a_debugger() {
    if !opt::enabled() {
        return;
    }
    opt::keep_variables();
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int { lo: 0, hi: 99 });
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.local("x", int, 8, 8);
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let a = fb.addr(AddrSpace::Frame, 0);
    let k = fb.int(5);
    fb.emit(Inst::Store {
        addr: a,
        src: k,
        w: Width::B8,
    });
    let r = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: r,
        addr: a,
        w: Width::B8,
    });
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    assert!(
        code.iter().any(|i| matches!(i, Inst::Store { .. })),
        "{code:?}"
    );
}

#[test]
fn statement_markers_stay_for_a_debugger() {
    if !opt::enabled() {
        return;
    }
    opt::keep_variables();
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::VarAssign, Span::new(FileId(0), 0, 1));
    let k = fb.int(5);
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    fb.emit(Inst::Ret { v: Some(k) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    assert_eq!(
        code.iter().filter(|i| matches!(i, Inst::Stmt)).count(),
        2,
        "{code:?}"
    );
}
