//! The optimiser (`opt.rs`): what it removes, and what it must leave alone.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

/// The code of function `f` after `ProgramBuilder::finish` (which optimises it).
fn build(kind: FuncKind, body: impl FnOnce(&mut FuncBuilder<'_>)) -> Vec<Inst> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    body(&mut fb);
    fb.finish();
    let p = pb.finish().expect("optimised code verifies");
    p.func(f).expect("f").code.clone()
}

fn count(code: &[Inst], pred: impl Fn(&Inst) -> bool) -> usize {
    code.iter().filter(|i| pred(i)).count()
}

fn on() -> bool {
    opt::enabled()
}

fn load(fb: &mut FuncBuilder<'_>, off: u32) -> Reg {
    let a = fb.addr(AddrSpace::Frame, off);
    let d = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: d,
        addr: a,
        w: Width::B8,
    });
    d
}

fn bin(fb: &mut FuncBuilder<'_>, op: IntOp, a: Reg, b: Reg) -> Reg {
    let dst = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin { op, dst, a, b });
    dst
}

#[test]
fn constants_fold_and_branches_on_them_go() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let a = fb.int(6);
        let b = fb.int(7);
        let m = bin(fb, IntOp::Mul, a, b);
        let c = fb.reg(RegTy::I64);
        let k = fb.int(42);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: m,
            b: k,
        });
        let (t, e) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t, f: e });
        fb.at(t);
        fb.emit(Inst::Ret { v: Some(m) });
        fb.at(e);
        let what = fb.program().check_msg("no return", Dir::To);
        fb.emit(Inst::Unreachable { what });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        0,
        "{code:?}"
    );
    assert!(code.iter().any(|i| matches!(i, Inst::Const { v: 42, .. })));
}

#[test]
fn overflow_and_division_by_zero_are_left_to_run_time() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.stmt(StmtKind::Return, sp());
        let a = fb.int(i64::MAX);
        let one = fb.int(1);
        let zero = fb.int(0);
        let s = bin(fb, IntOp::Add, a, one);
        let d = bin(fb, IntOp::Div, one, zero);
        let r = bin(fb, IntOp::Add, s, d);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { .. })),
        3,
        "{code:?}"
    );
}

#[test]
fn identities_use_the_operand() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let x = load(fb, 0);
        let one = fb.int(1);
        let zero = fb.int(0);
        let a = bin(fb, IntOp::Mul, x, one);
        let b = bin(fb, IntOp::Add, a, zero);
        fb.emit(Inst::Ret { v: Some(b) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { .. })),
        0,
        "{code:?}"
    );
    let Some(Inst::Load { dst, .. }) = code.iter().find(|i| matches!(i, Inst::Load { .. })) else {
        panic!("{code:?}");
    };
    assert!(code.contains(&Inst::Ret { v: Some(*dst) }), "{code:?}");
}

#[test]
fn repeated_computations_and_loads_are_shared() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        let x = load(fb, 0);
        let y = load(fb, 0);
        let s = bin(fb, IntOp::Add, x, y);
        let t = bin(fb, IntOp::Add, x, y);
        let r = bin(fb, IntOp::Mul, s, t);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Load { .. })),
        1,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { op: IntOp::Add, .. })),
        1
    );
}

/// `f` stores 5 to a frame slot, loads it, runs `between` and loads again; the number of
/// loads left.
fn loads_left(named: bool, between: impl FnOnce(&mut FuncBuilder<'_>, FuncId)) -> usize {
    let mut pb = ProgramBuilder::new();
    let g = pb.declare("g", FuncKind::Procedure, None);
    let mut gb = pb.define(g);
    gb.stmt(StmtKind::Return, sp());
    gb.emit(Inst::Ret { v: None });
    gb.finish();
    let int = pb.ty(Ty::Int { lo: 0, hi: 99 });
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    if named {
        fb.local("x", int, 8, 8);
    } else {
        fb.slot(8, 8);
    }
    fb.stmt(StmtKind::Return, sp());
    let v = fb.reg(RegTy::I64);
    let a = fb.addr(AddrSpace::Frame, 0);
    let k = fb.int(5);
    fb.emit(Inst::Move { dst: v, src: k });
    fb.emit(Inst::Store {
        addr: a,
        src: v,
        w: Width::B8,
    });
    let x = load(&mut fb, 0);
    between(&mut fb, g);
    let y = load(&mut fb, 0);
    let r = bin(&mut fb, IntOp::Add, x, y);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    count(code, |i| matches!(i, Inst::Load { .. }))
}

fn call(fb: &mut FuncBuilder<'_>, g: FuncId) {
    let args = fb.regs(&[]);
    fb.emit(Inst::Call {
        func: g,
        args,
        dst: None,
    });
}

#[test]
fn a_store_forwards_to_the_load() {
    if !on() {
        return;
    }
    // Both loads are the stored constant; a slot nobody can reach survives a call.
    assert_eq!(loads_left(false, |_, _| {}), 0);
    assert_eq!(loads_left(false, call), 0);
}

#[test]
fn a_call_or_a_statement_stops_what_a_debugger_or_callee_may_change() {
    if !on() {
        return;
    }
    // A scalar local can be written by a debugger at any statement or while a callee runs.
    assert_eq!(loads_left(true, call), 1);
    assert_eq!(
        loads_left(true, |fb, _| {
            fb.stmt(StmtKind::Null, sp());
        }),
        1
    );
}

#[test]
fn a_passing_check_goes_and_a_failing_one_stays() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.stmt(StmtKind::Return, sp());
        let what = fb.program().check_msg("range", Dir::To);
        let (v, lo, hi) = (fb.int(5), fb.int(0), fb.int(9));
        fb.emit(Inst::Check {
            kind: CheckKind::Range,
            v,
            lo,
            hi,
            what,
        });
        let big = fb.int(50);
        fb.emit(Inst::Check {
            kind: CheckKind::Range,
            v: big,
            lo,
            hi,
            what,
        });
        fb.emit(Inst::Ret { v: Some(v) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Check { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn nothing_is_shared_across_a_suspension() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Process, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Wait, sp());
        let t = fb.int(1);
        fb.emit(Inst::WaitFor { t });
        fb.emit(Inst::Suspend { resume: 0 });
        let x = load(fb, 0);
        let one = fb.int(1);
        let y = bin(fb, IntOp::Mul, x, one);
        let z = bin(fb, IntOp::Add, y, y);
        let addr = fb.addr(AddrSpace::Frame, 8);
        fb.emit(Inst::Store {
            addr,
            src: z,
            w: Width::B8,
        });
        fb.emit(Inst::Br { to: BlockId(0) });
    });
    // The verifier ran on this code (`build` unwraps it); the multiplication by one is gone.
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { op: IntOp::Mul, .. })),
        0,
        "{code:?}"
    );
}

#[test]
fn statement_locations_follow_their_instructions() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::VarAssign, sp());
    let x = load(&mut fb, 0);
    let s2 = fb.stmt(StmtKind::Return, sp());
    let one = fb.int(1);
    let r = bin(&mut fb, IntOp::Mul, x, one);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let func = p.func(f).expect("f");
    assert_eq!(func.code.len(), func.loc.len());
    let ret = func
        .code
        .iter()
        .position(|i| matches!(i, Inst::Ret { .. }))
        .expect("ret");
    assert_eq!(func.loc[ret], s2);
}

#[test]
fn facts_do_not_leak_between_sibling_branches_or_into_a_join() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::If, sp());
        let x = load(fb, 0);
        let zero = fb.int(0);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Gt,
            dst: c,
            a: x,
            b: zero,
        });
        let (bt, be, bj) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t: bt, f: be });
        fb.at(bt);
        // Shared with the load before the branch: only this branch can reach here.
        let y = load(fb, 0);
        let m = fb.reg(RegTy::I64);
        fb.emit(Inst::Move { dst: m, src: y });
        fb.emit(Inst::Br { to: bj });
        fb.at(be);
        let a = fb.addr(AddrSpace::Frame, 0);
        let seven = fb.int(7);
        fb.emit(Inst::Store {
            addr: a,
            src: seven,
            w: Width::B8,
        });
        fb.emit(Inst::Move { dst: m, src: seven });
        fb.emit(Inst::Br { to: bj });
        fb.at(bj);
        // Reached from both: the store of the other branch means this must read memory.
        let z = load(fb, 0);
        let r = bin(fb, IntOp::Add, z, m);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Load { .. })),
        2,
        "{code:?}"
    );
}
