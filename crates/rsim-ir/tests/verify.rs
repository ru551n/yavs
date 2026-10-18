//! The verifier rejects malformed IR; the dump is stable text.

use rsim_diag::{FileId, Span};
use rsim_ir::*;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

/// Build one function with `body` and return the verifier messages.
fn errors(kind: FuncKind, body: impl FnOnce(&mut FuncBuilder<'_>)) -> Vec<String> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    body(&mut fb);
    fb.finish();
    match pb.finish() {
        Ok(_) => Vec::new(),
        Err(e) => e.into_iter().map(|e| e.msg).collect(),
    }
}

fn expect(kind: FuncKind, want: &str, body: impl FnOnce(&mut FuncBuilder<'_>)) {
    let e = errors(kind, body);
    assert!(
        e.iter().any(|m| m.contains(want)),
        "want {want:?}, got {e:#?}"
    );
}

#[test]
fn well_formed_passes() {
    let e = errors(FuncKind::Process, |fb| {
        fb.stmt(StmtKind::Wait, sp());
        let t = fb.int(5);
        fb.emit(Inst::WaitFor { t });
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: BlockId(0) });
    });
    assert!(e.is_empty(), "{e:#?}");
}

#[test]
fn malformed_ir_is_rejected() {
    expect(FuncKind::Process, "does not end with a terminator", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.int(1);
    });
    expect(FuncKind::Process, "terminator in the middle", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.emit(Inst::Ret { v: None });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Process, "expected Some(F64)", |fb| {
        fb.stmt(StmtKind::Null, sp());
        let i = fb.int(1);
        let d = fb.reg(RegTy::F64);
        fb.emit(Inst::FUn {
            op: FUnOp::Neg,
            dst: d,
            a: i,
        });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Process, "branch to missing block", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.emit(Inst::Br { to: BlockId(9) });
    });
    expect(FuncKind::Process, "elaboration-only instruction", |fb| {
        fb.stmt(StmtKind::Null, sp());
        let args = fb.regs(&[]);
        fb.emit(Inst::Elab {
            op: ElabOp::Signal,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Function, "cannot suspend", |fb| {
        fb.stmt(StmtKind::Wait, sp());
        fb.emit(Inst::Suspend { resume: 0 });
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    expect(FuncKind::Process, "live across a suspension point", |fb| {
        fb.stmt(StmtKind::Wait, sp());
        let t = fb.int(5);
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::WaitFor { t });
        fb.emit(Inst::Ret { v: None });
    });
    expect(
        FuncKind::Process,
        "instruction before any statement",
        |fb| {
            fb.emit(Inst::Ret { v: None });
        },
    );
    expect(FuncKind::Procedure, "return value does not match", |fb| {
        fb.stmt(StmtKind::Return, sp());
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
}

#[test]
fn calls_are_checked() {
    let mut pb = ProgramBuilder::new();
    let w = pb.declare("waits", FuncKind::Procedure, None);
    let f = pb.declare("fun", FuncKind::Function, None);
    let mut fb = pb.define(w);
    fb.param("x", Width::B8, ParamClass::Value, Mode::In);
    fb.stmt(StmtKind::Wait, sp());
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Ret { v: None });
    fb.finish();
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::Call, sp());
    let args = fb.regs(&[]);
    fb.emit(Inst::Call {
        func: w,
        args,
        dst: None,
    });
    let z = fb.int(0);
    fb.emit(Inst::Ret { v: Some(z) });
    fb.finish();
    let e: Vec<String> = pb
        .finish()
        .unwrap_err()
        .into_iter()
        .map(|e| e.msg)
        .collect();
    assert!(
        e.iter().any(|m| m.contains("takes 1 arguments, 0 given")),
        "{e:#?}"
    );
    assert!(e.iter().any(|m| m.contains("which may suspend")), "{e:#?}");
}

#[test]
fn undefined_function() {
    let mut pb = ProgramBuilder::new();
    pb.declare("ghost", FuncKind::Function, None);
    let e = pb.finish().unwrap_err();
    assert_eq!(e[0].msg, "declared but never defined");
}

#[test]
fn flags_and_dump() {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int { lo: 0, hi: 9 });
    let tick = pb.declare("tick", FuncKind::Procedure, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(tick);
    fb.stmt(StmtKind::Wait, sp());
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Ret { v: None });
    fb.finish();
    let mut fb = pb.define(p);
    fb.local("v", int, 8, 8);
    fb.stmt(StmtKind::Call, sp());
    let args = fb.regs(&[]);
    fb.emit(Inst::Call {
        func: tick,
        args,
        dst: None,
    });
    fb.stmt(StmtKind::VarAssign, sp());
    let a = fb.addr(AddrSpace::Shared, 0);
    let one = fb.int(1);
    fb.emit(Inst::Store {
        addr: a,
        src: one,
        w: Width::B8,
    });
    fb.emit(Inst::Br { to: BlockId(0) });
    fb.finish();
    let prog = pb.finish().unwrap();
    let pf = prog.func(p).unwrap();
    assert!(pf.flags.contains(FuncFlags::MAY_SUSPEND));
    assert!(prog.is_serial(p));
    assert!(!prog.is_serial(tick));
    assert_eq!(pf.resume, [2]);
    assert_eq!(
        prog.dump_func(p),
        "f1 Process `p` frame 8 regs 2 flags 0x3
b0:
     0 s1    Stmt
     1 s1    Call { func: f0, args: #0, dst: None } []
     2 s2    Stmt
     3 s2    AddrOf { dst: r0, space: Shared, off: 0 }
     4 s2    Const { dst: r1, v: 1 }
     5 s2    Store { addr: r0, src: r1, w: B8 }
     6 s2    Br { to: b0 }
"
    );
}
