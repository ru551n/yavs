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

/// Store the address of slot `off` into the slot after it, so that the slot is not promoted
/// to a register: the tests of load sharing need the loads to stay.
fn pin(fb: &mut FuncBuilder<'_>, off: u32) {
    let a = fb.addr(AddrSpace::Frame, off);
    let b = fb.addr(AddrSpace::Frame, off + 8);
    fb.emit(Inst::Store {
        addr: b,
        src: a,
        w: Width::B8,
    });
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
        pin(fb, 0);
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
        pin(fb, 0);
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
    // A scalar local whose address is taken can be written by a callee, and a debugger may
    // write it at any statement: its loads stay. (Without a debugger the unreachable ones
    // live in registers, see `a_variable_nobody_can_reach_lives_in_a_register`.)
    assert_eq!(loads_left(true, call), 0);
    assert_eq!(
        loads_left(true, |fb, _| {
            fb.stmt(StmtKind::Null, sp());
        }),
        0
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
        pin(fb, 0);
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

/// A function with one loop: `x := 0; while x < 10 loop x := x + 1; end loop; return x`, the
/// variable in frame slot 0 (a named local or an anonymous slot).
fn counter_loop(named: bool, escape: bool) -> (usize, usize) {
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
        fb.slot(16, 8);
    }
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    fb.stmt(StmtKind::VarAssign, sp());
    let a = fb.addr(AddrSpace::Frame, 0);
    let zero = fb.int(0);
    fb.emit(Inst::Store {
        addr: a,
        src: zero,
        w: Width::B8,
    });
    if escape {
        pin(&mut fb, 0);
    }
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.stmt(StmtKind::If, sp());
    let x = load(&mut fb, 0);
    let ten = fb.int(10);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Lt,
        dst: c,
        a: x,
        b: ten,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    fb.stmt(StmtKind::VarAssign, sp());
    let x = load(&mut fb, 0);
    let one = fb.int(1);
    let y = bin(&mut fb, IntOp::Add, x, one);
    let a = fb.addr(AddrSpace::Frame, 0);
    fb.emit(Inst::Store {
        addr: a,
        src: y,
        w: Width::B8,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.stmt(StmtKind::Return, sp());
    let r = load(&mut fb, 0);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    (
        count(code, |i| matches!(i, Inst::Load { .. })),
        count(code, |i| matches!(i, Inst::Store { .. })),
    )
}

#[test]
fn a_variable_nobody_can_reach_lives_in_a_register() {
    if !on() {
        return;
    }
    // Anonymous slot or named variable: no load or store is left.
    assert_eq!(counter_loop(false, false), (0, 0));
    assert_eq!(counter_loop(true, false), (0, 0));
}

#[test]
fn a_slot_whose_address_is_passed_stays_in_memory() {
    if !on() {
        return;
    }
    let (loads, stores) = counter_loop(false, true);
    assert!(loads >= 1 && stores >= 3, "{loads} loads {stores} stores");
}

#[test]
fn a_marker_survives_only_for_a_statement_with_temporaries() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::VarAssign, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let one = fb.int(1);
        let y = bin(fb, IntOp::Add, x, one);
        // The second statement allocates a temporary: its marker frees the first one's.
        fb.stmt(StmtKind::VarAssign, sp());
        let n = fb.int(8);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: a, size: n });
        fb.emit(Inst::Store {
            addr: a,
            src: y,
            w: Width::B8,
        });
        fb.stmt(StmtKind::Return, sp());
        let z = bin(fb, IntOp::Add, y, one);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    assert_eq!(count(&code, |i| matches!(i, Inst::Stmt)), 1, "{code:?}");
    let alloc = code
        .iter()
        .position(|i| matches!(i, Inst::Alloc { .. }))
        .expect("alloc");
    assert!(
        code[..alloc].iter().any(|i| matches!(i, Inst::Stmt)),
        "{code:?}"
    );
}

#[test]
fn equal_constants_share_a_register_and_unused_registers_go() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Return, sp());
    pin(&mut fb, 0);
    for _ in 0..20 {
        fb.reg(RegTy::I64);
    }
    let x = load(&mut fb, 0);
    let (a, b) = (fb.int(7), fb.int(7));
    let s = bin(&mut fb, IntOp::Sub, a, x);
    let t = bin(&mut fb, IntOp::Sub, b, s);
    fb.emit(Inst::Ret { v: Some(t) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let func = p.func(f).expect("f");
    assert_eq!(
        count(&func.code, |i| matches!(i, Inst::Const { v: 7, .. })),
        1,
        "{:?}",
        func.code
    );
    assert!(func.reg_ty.len() < 12, "{} registers", func.reg_ty.len());
}

/// A subprogram with a local array `[data, dope, handle, descriptor]` (slot 0): the object
/// is made by `New`, its handle stored at slot 16 and dereferenced; `leak` also passes the
/// handle to a call. Returns the code of the function.
fn local_array(leak: bool, shared: bool, kind: FuncKind) -> Vec<Inst> {
    let mut pb = ProgramBuilder::new();
    let g = pb.declare("g", FuncKind::Procedure, None);
    let mut gb = pb.define(g);
    gb.param("h", Width::B8, ParamClass::Value, Mode::In, None);
    gb.stmt(StmtKind::Return, sp());
    gb.emit(Inst::Ret { v: None });
    gb.finish();
    let elem = pb.ty(Ty::Int { lo: 0, hi: 255 });
    let arr = pb.ty(Ty::Array {
        elem,
        elem_desc: None,
        dims: 1,
    });
    let what = pb.check_msg("object", Dir::To);
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    let off = fb.local("v", arr, 56, 8);
    fb.root(off + 16, RootKind::Handle);
    fb.stmt(StmtKind::Decl, sp());
    let n = fb.int(32);
    let buf = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: buf, size: n });
    let h = fb.reg(RegTy::Handle);
    fb.emit(Inst::New {
        dst: h,
        ty: arr,
        init: buf,
        size: n,
        shared,
    });
    let hs = fb.addr(AddrSpace::Frame, off + 16);
    fb.emit(Inst::Store {
        addr: hs,
        src: h,
        w: Width::B8,
    });
    let p = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Deref {
        dst: p,
        handle: h,
        what,
    });
    let ds = fb.addr(AddrSpace::Frame, off);
    fb.emit(Inst::Store {
        addr: ds,
        src: p,
        w: Width::B8,
    });
    if leak {
        let args = fb.regs(&[h]);
        fb.emit(Inst::Call {
            func: g,
            args,
            dst: None,
        });
    }
    fb.stmt(StmtKind::Return, sp());
    fb.emit(Inst::Ret {
        v: (kind == FuncKind::Function).then_some(n),
    });
    fb.finish();
    let p = pb.finish().expect("verifies");
    p.func(f).expect("f").code.clone()
}

#[test]
fn a_local_array_lives_in_the_frames_temporary_memory() {
    if !on() {
        return;
    }
    let is_new = |i: &Inst| matches!(i, Inst::New { .. } | Inst::NewFill { .. });
    let frame = |i: &Inst| matches!(i, Inst::NewFrame { .. });
    let code = local_array(false, false, FuncKind::Function);
    assert_eq!(count(&code, is_new), 0);
    assert_eq!(count(&code, frame), 1);
    assert_eq!(count(&code, |i| matches!(i, Inst::Deref { .. })), 0);
    // A handle somebody else gets, a shared heap and a process keep the heap object.
    for code in [
        local_array(true, false, FuncKind::Function),
        local_array(false, true, FuncKind::Function),
        local_array(false, false, FuncKind::Process),
    ] {
        assert_eq!(count(&code, frame), 0);
        assert_eq!(count(&code, is_new), 1);
    }
}

/// A byte read from frame slot 0 (any value 0 to 255), and `pin` so that it stays a load.
fn byte(fb: &mut FuncBuilder<'_>) -> Reg {
    let a = fb.addr(AddrSpace::Frame, 0);
    pin(fb, 0);
    let d = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: d,
        addr: a,
        w: Width::B1,
    });
    d
}

fn check(fb: &mut FuncBuilder<'_>, v: Reg, lo: i64, hi: i64) {
    let what = fb.program().check_msg("range", Dir::To);
    let (lo, hi) = (fb.int(lo), fb.int(hi));
    fb.emit(Inst::Check {
        kind: CheckKind::Range,
        v,
        lo,
        hi,
        what,
    });
}

#[test]
fn a_check_the_value_range_proves_goes() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        let v = byte(fb);
        // A byte is 0 to 255: the first check passes, the second may fail.
        check(fb, v, 0, 255);
        check(fb, v, 0, 100);
        fb.emit(Inst::Ret { v: Some(v) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Check { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn a_range_checked_sum_that_cannot_leave_its_range_is_plain() {
    if !on() {
        return;
    }
    let sums = |hi: i64| {
        let code = build(FuncKind::Function, |fb| {
            fb.slot(32, 8);
            fb.stmt(StmtKind::Return, sp());
            let v = byte(fb);
            let one = fb.int(1);
            let (l, h) = (fb.int(0), fb.int(hi));
            let what = fb.program().check_msg("range", Dir::To);
            let d = fb.reg(RegTy::I64);
            fb.emit(Inst::BinRange {
                op: IntOp::Add,
                dst: d,
                a: v,
                b: one,
                lo: l,
                hi: h,
                what,
            });
            fb.emit(Inst::Ret { v: Some(d) });
        });
        count(&code, |i| matches!(i, Inst::BinRange { .. }))
    };
    assert_eq!(sums(256), 0);
    assert_eq!(sums(255), 1);
}

#[test]
fn a_length_plus_one_is_not_zero() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        let d = fb.addr(AddrSpace::Frame, 0);
        pin(fb, 0);
        let len = fb.reg(RegTy::I64);
        fb.emit(Inst::DopeLen {
            dst: len,
            dope: d,
            dim: 0,
        });
        let one = fb.int(1);
        let n = bin(fb, IntOp::Add, len, one);
        let zero = fb.int(0);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: n,
            b: zero,
        });
        let (t, e) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t, f: e });
        fb.at(t);
        fb.emit(Inst::Ret { v: Some(zero) });
        fb.at(e);
        fb.emit(Inst::Ret { v: Some(n) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Cmp { .. })),
        0,
        "{code:?}"
    );
}

#[test]
fn a_branch_narrows_the_compared_value_on_each_edge() {
    if !on() {
        return;
    }
    // x is any 64-bit value; `if x < 10 then check x <= 9` passes, the same check in the
    // else branch may fail.
    let checks = |in_then: bool| {
        let code = build(FuncKind::Function, |fb| {
            fb.slot(32, 8);
            fb.stmt(StmtKind::Return, sp());
            let a = fb.addr(AddrSpace::Frame, 0);
            pin(fb, 0);
            let x = fb.reg(RegTy::I64);
            fb.emit(Inst::Load {
                dst: x,
                addr: a,
                w: Width::B8,
            });
            let ten = fb.int(10);
            let c = fb.reg(RegTy::I64);
            fb.emit(Inst::Cmp {
                op: CmpOp::Lt,
                dst: c,
                a: x,
                b: ten,
            });
            let (t, e) = (fb.block(), fb.block());
            fb.emit(Inst::CondBr { c, t, f: e });
            fb.at(t);
            if in_then {
                check(fb, x, i64::MIN, 9);
            }
            fb.emit(Inst::Ret { v: Some(x) });
            fb.at(e);
            if !in_then {
                check(fb, x, i64::MIN, 9);
            }
            fb.emit(Inst::Ret { v: Some(x) });
        });
        count(&code, |i| matches!(i, Inst::Check { .. }))
    };
    assert_eq!(checks(true), 0);
    assert_eq!(checks(false), 1);
}

/// A pointer written to slot 0, which a root map covers as an address, and read twice. Returns
/// the stores left to the root slot (none when it is promoted). The register type of the stored value decides: a `Ptr` register is a root
/// of the collector as the slot is, an integer one is not.
fn root_slot_stores(handle: bool, ty: RegTy) -> usize {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(32, 8);
    fb.root(
        0,
        if handle {
            RootKind::Handle
        } else {
            RootKind::Addr
        },
    );
    fb.stmt(StmtKind::VarAssign, sp());
    let n = fb.int(8);
    let v = fb.reg(ty);
    if ty == RegTy::Handle {
        let z = fb.int(0);
        fb.emit(Inst::Move { dst: v, src: z });
    } else {
        let p = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: p, size: n });
        fb.emit(Inst::Move { dst: v, src: p });
    }
    let a = fb.addr(AddrSpace::Frame, 0);
    fb.emit(Inst::Store {
        addr: a,
        src: v,
        w: Width::B8,
    });
    fb.stmt(StmtKind::Return, sp());
    // Two reads of the slot, each kept in a slot of its own.
    for to in [16, 24] {
        let a = fb.addr(AddrSpace::Frame, 0);
        let x = fb.reg(ty);
        fb.emit(Inst::Load {
            dst: x,
            addr: a,
            w: Width::B8,
        });
        let t = fb.addr(AddrSpace::Frame, to);
        fb.emit(Inst::Store {
            addr: t,
            src: x,
            w: Width::B8,
        });
    }
    let zero = fb.int(0);
    fb.emit(Inst::Ret { v: Some(zero) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    let slot0: Vec<Reg> = code
        .iter()
        .filter_map(|i| match *i {
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off: 0,
            } => Some(dst),
            _ => None,
        })
        .collect();
    count(
        code,
        |i| matches!(i, Inst::Store { addr, .. } if slot0.contains(addr)),
    )
}

#[test]
fn a_written_root_slot_lives_in_a_register_of_the_roots_type() {
    if !on() {
        return;
    }
    // A pointer into an address slot: promoted. A handle slot is promoted for a handle.
    assert_eq!(root_slot_stores(false, RegTy::Ptr), 0);
    assert_eq!(root_slot_stores(true, RegTy::Handle), 0);
    // An integer register is not a root: the value stays where the collector looks.
    assert_eq!(root_slot_stores(false, RegTy::I64), 1);
}
