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

/// A statement that makes a temporary: `Alloc` of 8 bytes and a store into it, kept as a root
/// (a block that is only stored into is dead and goes).
fn make_temporary(fb: &mut FuncBuilder<'_>) {
    fb.stmt(StmtKind::VarAssign, sp());
    let n = fb.int(8);
    let a = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: a, size: n });
    fb.emit(Inst::Store {
        addr: a,
        src: n,
        w: Width::B8,
    });
    fb.emit(Inst::TempRoot {
        addr: a,
        desc: None,
        ty: None,
    });
}

#[test]
fn a_marker_survives_only_where_it_frees_a_temporary() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        // Nothing before the first statement, so no marker; the second statement's marker frees
        // the first one's temporary; the last statement makes none.
        make_temporary(fb);
        make_temporary(fb);
        fb.stmt(StmtKind::Return, sp());
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    assert_eq!(count(&code, |i| matches!(i, Inst::Stmt)), 1, "{code:?}");
    let (first, second) = {
        let mut at = code
            .iter()
            .enumerate()
            .filter(|(_, i)| matches!(i, Inst::Alloc { .. }))
            .map(|(k, _)| k);
        (at.next().expect("alloc"), at.next().expect("alloc"))
    };
    let stmt = code
        .iter()
        .position(|i| matches!(i, Inst::Stmt))
        .expect("marker");
    assert!(first < stmt && stmt < second, "{code:?}");
}

#[test]
fn a_loop_keeps_the_marker_that_frees_its_temporaries() {
    if !on() {
        return;
    }
    // The temporary of the body is freed by the marker at its start on the next round.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::VarAssign, sp());
        pin(fb, 0);
        let (head, done) = (fb.block(), fb.block());
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        make_temporary(fb);
        let c = load(fb, 0);
        fb.emit(Inst::CondBr {
            c,
            t: head,
            f: done,
        });
        fb.at(done);
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    assert_eq!(count(&code, |i| matches!(i, Inst::Stmt)), 1, "{code:?}");
}

/// `f` calls `g` (which returns a register of type `res` and allocates nothing) in a statement
/// of its own between two statements that make a temporary; the markers left in `f`.
fn markers_around_a_call(res: RegTy) -> usize {
    let mut pb = ProgramBuilder::new();
    let g = pb.declare("g", FuncKind::Function, None);
    let f = pb.declare("f", FuncKind::Function, None);
    let mut gb = pb.define(g);
    gb.slot(16, 8);
    gb.stmt(StmtKind::Return, sp());
    let r = gb.reg(res);
    if res == RegTy::Ptr {
        gb.emit(Inst::AddrOf {
            dst: r,
            space: AddrSpace::Frame,
            off: 0,
        });
    } else {
        gb.emit(Inst::Const { dst: r, v: 7 });
    }
    gb.emit(Inst::Ret { v: Some(r) });
    gb.finish();
    let mut fb = pb.define(f);
    make_temporary(&mut fb);
    fb.stmt(StmtKind::VarAssign, sp());
    let args = fb.regs(&[]);
    let d = fb.reg(res);
    fb.emit(Inst::Call {
        func: g,
        args,
        dst: Some(d),
    });
    make_temporary(&mut fb);
    fb.stmt(StmtKind::Return, sp());
    let z = fb.int(0);
    fb.emit(Inst::Ret { v: Some(z) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    count(&p.func(f).expect("f").code, |i| matches!(i, Inst::Stmt))
}

#[test]
fn a_call_that_leaves_no_temporaries_needs_no_marker() {
    if !on() {
        return;
    }
    // A scalar result of a function that allocates nothing: the one marker left is the second
    // temporary's, which frees the first. An address result may be a composite the callee built
    // in the caller's temporaries: the call's own statement keeps its marker too.
    assert_eq!(markers_around_a_call(RegTy::I64), 1);
    assert!(markers_around_a_call(RegTy::Ptr) > 1);
}

/// A statement calling the native `symbol` (a scalar result) between two that make a temporary:
/// the markers left.
fn markers_around_a_native(symbol: &str, nargs: usize) -> usize {
    let code = build(FuncKind::Procedure, |fb| {
        make_temporary(fb);
        fb.stmt(StmtKind::VarAssign, sp());
        let native = fb.program().native(symbol, None);
        let regs: Vec<Reg> = (0..nargs).map(|i| fb.int(i as i64)).collect();
        let args = fb.regs(&regs);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: Some(dst),
        });
        make_temporary(fb);
        fb.stmt(StmtKind::Return, sp());
        fb.emit(Inst::Ret { v: None });
    });
    count(&code, |i| matches!(i, Inst::Stmt))
}

#[test]
fn a_native_that_may_hand_the_call_back_keeps_its_marker() {
    if !on() {
        return;
    }
    // The call of a native with a scalar result makes no temporary, except an `ieee` one that
    // can give the call back to its VHDL body (it builds descriptors for its array arguments).
    assert_eq!(markers_around_a_native("yavs.implicit.eq.integer", 2), 1);
    assert_eq!(
        markers_around_a_native("yavs.ieee.numeric_std.to_integer(unsigned)integer", 4),
        1
    );
    assert!(markers_around_a_native("yavs.ieee.numeric_std.to_integer(unsigned)integer@9", 4) > 1);
}

#[test]
fn a_statement_that_only_reads_signals_needs_no_marker() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Procedure, |fb| {
        for _ in 0..3 {
            fb.stmt(StmtKind::VarAssign, sp());
            let s = fb.reg(RegTy::Sig);
            let (off, len) = (fb.int(0), fb.int(1));
            fb.emit(Inst::SigVal {
                dst: s,
                sig: 0,
                offset: off,
                len,
            });
            let d = fb.reg(RegTy::I64);
            fb.emit(Inst::SigReadScalar { dst: d, sig: s });
        }
        // The last statement allocates a temporary: nothing before it left one to free.
        make_temporary(fb);
        fb.stmt(StmtKind::Return, sp());
        fb.emit(Inst::Ret { v: None });
    });
    assert_eq!(count(&code, |i| matches!(i, Inst::Stmt)), 0, "{code:?}");
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

#[test]
fn registers_with_disjoint_lives_share_a_cell() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(16 * 10 + 8, 8);
    fb.stmt(StmtKind::Return, sp());
    // Ten sums of two loads, each stored before the next starts: the values of different
    // rounds are never live together.
    let mut made = 0;
    for k in 0..10 {
        pin(&mut fb, 16 * k);
        let (x, y) = (load(&mut fb, 16 * k), load(&mut fb, 16 * k + 8));
        let s = bin(&mut fb, IntOp::Add, x, y);
        let a = fb.addr(AddrSpace::Frame, 16 * k);
        fb.emit(Inst::Store {
            addr: a,
            src: s,
            w: Width::B8,
        });
        made += 3;
    }
    let r = fb.int(0);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let func = p.func(f).expect("f");
    assert_eq!(count(&func.code, |i| matches!(i, Inst::Load { .. })), 20);
    // The addresses are preloaded cells of their own; the loaded values and sums share.
    let addrs = count(&func.code, |i| matches!(i, Inst::AddrOf { .. }));
    assert!(
        func.reg_ty.len() < addrs + made / 2,
        "{} registers for {made} values and {addrs} addresses",
        func.reg_ty.len()
    );
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

/// Make a function long enough and give it a loop, the two conditions of the passes that
/// leave short and loop-free functions alone (the padding's results are unused and go). The
/// code after the call is in the block after the loop.
fn pad(fb: &mut FuncBuilder<'_>) {
    for k in 0..40 {
        let a = fb.reg(RegTy::I64);
        fb.emit(Inst::Const { dst: a, v: k });
    }
    let (head, rest) = (fb.block(), fb.block());
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let a = fb.addr(AddrSpace::Frame, 16);
    pin(fb, 16);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: c,
        addr: a,
        w: Width::B1,
    });
    fb.emit(Inst::CondBr {
        c,
        t: head,
        f: rest,
    });
    fb.at(rest);
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
        pad(fb);
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
            pad(fb);
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
        pad(fb);
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
    // The loop's own branch stays; the test of the length does not.
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        1,
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
            pad(fb);
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

/// A descriptor `[left, right, dir]` in a fresh temporary.
fn made(fb: &mut FuncBuilder<'_>, left: i64, right: i64, dir: i64) -> Reg {
    let at = fb.reg(RegTy::Ptr);
    let n = fb.int(24);
    fb.emit(Inst::Alloc { dst: at, size: n });
    let parts: Vec<Reg> = [left, right, dir].iter().map(|&v| fb.int(v)).collect();
    let parts = fb.regs(&parts);
    fb.emit(Inst::MakeDope { at, parts });
    at
}

#[test]
fn a_descriptor_of_constants_has_known_lengths() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let a = made(fb, 14, 0, 1);
        let b = made(fb, 0, 14, 0);
        let c = made(fb, 0, 3, 0);
        let what = fb.program().check_msg("length", Dir::To);
        // Equal lengths of opposite directions: the check passes and goes.
        fb.emit(Inst::SubtypeCheck {
            dope: a,
            want: b,
            dims: 1,
            what,
        });
        let n = fb.reg(RegTy::I64);
        fb.emit(Inst::DopeLen {
            dst: n,
            dope: a,
            dim: 0,
        });
        // Different lengths: stays.
        fb.emit(Inst::SubtypeCheck {
            dope: a,
            want: c,
            dims: 1,
            what,
        });
        fb.emit(Inst::Ret { v: Some(n) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::DopeLen { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::SubtypeCheck { .. })),
        1,
        "{code:?}"
    );
    assert!(
        code.iter().any(|i| matches!(i, Inst::Const { v: 15, .. })),
        "{code:?}"
    );
}

#[test]
fn a_descriptor_nobody_reads_goes_with_its_block() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let a = made(fb, 0, 3, 0);
        let n = fb.reg(RegTy::I64);
        fb.emit(Inst::DopeLen {
            dst: n,
            dope: a,
            dim: 0,
        });
        fb.emit(Inst::Ret { v: Some(n) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::MakeDope { .. })),
        0,
        "{code:?}"
    );
}

/// The constant `Ret` returns, if the code ends in a `Ret` of a register a `Const` defines.
fn returned_const(code: &[Inst]) -> Option<i64> {
    let r = code.iter().find_map(|i| match i {
        Inst::Ret { v: Some(r) } => Some(*r),
        _ => None,
    })?;
    code.iter().find_map(|i| match i {
        Inst::Const { dst, v } if *dst == r => Some(*v),
        _ => None,
    })
}

#[test]
fn a_slice_whose_bounds_differ_by_a_constant_has_that_length() {
    if !on() {
        return;
    }
    // The length of `(x, x + c, dir)` through a checked addition: `c + 1` to, `c + 1` downto
    // with the bounds the other way round, and 0 when the range is null.
    let len = |c: i64, dir: i64, swap: bool| {
        let code = build(FuncKind::Function, |fb| {
            fb.slot(16, 8);
            fb.stmt(StmtKind::Return, sp());
            pin(fb, 0);
            let x = load(fb, 0);
            let (k, lo, hi) = (fb.int(c), fb.int(-1000), fb.int(1000));
            let what = fb.program().check_msg("range", Dir::To);
            let r = fb.reg(RegTy::I64);
            fb.emit(Inst::BinRange {
                op: IntOp::Add,
                dst: r,
                a: x,
                b: k,
                lo,
                hi,
                what,
            });
            let size = fb.int(24);
            let dope = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Alloc { dst: dope, size });
            let d = fb.int(dir);
            let parts = if swap {
                fb.regs(&[r, x, d])
            } else {
                fb.regs(&[x, r, d])
            };
            fb.emit(Inst::MakeDope { at: dope, parts });
            let n = fb.reg(RegTy::I64);
            fb.emit(Inst::DopeLen {
                dst: n,
                dope,
                dim: 0,
            });
            fb.emit(Inst::Ret { v: Some(n) });
        });
        assert_eq!(
            count(&code, |i| matches!(i, Inst::DopeLen { .. })),
            0,
            "{code:?}"
        );
        assert_eq!(
            count(&code, |i| matches!(i, Inst::MakeDope { .. })),
            0,
            "{code:?}"
        );
        returned_const(&code)
    };
    assert_eq!(len(3, 0, false), Some(4));
    assert_eq!(len(3, 1, true), Some(4));
    assert_eq!(len(-1, 0, false), Some(0));
    assert_eq!(len(-5, 0, false), Some(0));
    assert_eq!(len(0, 0, false), Some(1));
}

#[test]
fn a_descriptor_from_zero_to_a_length_minus_one_has_that_length() {
    if !on() {
        return;
    }
    // `variable sv : vector (0 to sl - 1)` with `sl := value'length`: checking `sv` against the
    // parameter's descriptor decides nothing; `1 to sl - 1` is another length.
    let checks = |left: i64| {
        let code = build(FuncKind::Procedure, |fb| {
            // (The parameter's pointer and the descriptor are slots of their own.)
            let int = fb.program().ty(Ty::Int { lo: 0, hi: 99 });
            fb.local("p", int, 8, 8);
            fb.slot(16, 8);
            fb.local("d", int, 24, 8);
            fb.stmt(StmtKind::Return, sp());
            pin(fb, 0);
            let a = fb.addr(AddrSpace::Frame, 0);
            let p = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Load {
                dst: p,
                addr: a,
                w: Width::B8,
            });
            let x = fb.reg(RegTy::I64);
            fb.emit(Inst::DopeLen {
                dst: x,
                dope: p,
                dim: 0,
            });
            let one = fb.int(1);
            let r = bin(fb, IntOp::Sub, x, one);
            // (The descriptor of a local array lives in its slot.)
            let at = fb.addr(AddrSpace::Frame, 24);
            let (l, d) = (fb.int(left), fb.int(0));
            let parts = fb.regs(&[l, r, d]);
            fb.emit(Inst::MakeDope { at, parts });
            let what = fb.program().check_msg("length", Dir::To);
            fb.emit(Inst::SubtypeCheck {
                dope: at,
                want: p,
                dims: 1,
                what,
            });
            fb.emit(Inst::Ret { v: None });
        });
        count(&code, |i| matches!(i, Inst::SubtypeCheck { .. }))
    };
    assert_eq!(checks(0), 0);
    assert_eq!(checks(1), 1);
}

#[test]
fn a_block_only_stored_into_goes() {
    if !on() {
        return;
    }
    // The pair of an inlined table read once its words were read back: nothing reads the block.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let size = fb.int(16);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: a, size });
        fb.emit(Inst::Store {
            addr: a,
            src: x,
            w: Width::B8,
        });
        let eight = fb.int(8);
        let b = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: b,
            base: a,
            pos: eight,
            stride: 1,
        });
        fb.emit(Inst::Store {
            addr: b,
            src: x,
            w: Width::B8,
        });
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    assert_eq!(
        count(&code, |i| matches!(
            i,
            Inst::Alloc { .. } | Inst::Index { .. }
        )),
        0,
        "{code:?}"
    );
    // (The store of `pin` is all that is left.)
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Store { .. })),
        1,
        "{code:?}"
    );
    // The same block handed to a call stays.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let size = fb.int(16);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: a, size });
        let seven = fb.int(7);
        fb.emit(Inst::Store {
            addr: a,
            src: seven,
            w: Width::B8,
        });
        let native = fb.program().native("yavs.implicit.eq.integer", None);
        let args = fb.regs(&[a, seven]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn registering_a_root_forgets_nothing_about_memory() {
    if !on() {
        return;
    }
    // A `TempRoot` writes no memory: the word stored in the block before it reads back.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let size = fb.int(16);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: a, size });
        fb.emit(Inst::Store {
            addr: a,
            src: x,
            w: Width::B8,
        });
        fb.emit(Inst::TempRoot {
            addr: a,
            desc: None,
            ty: None,
        });
        let y = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: y,
            addr: a,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: Some(y) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Load { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn a_position_repeated_with_another_message_is_shared() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let dope = fb.addr(AddrSpace::Inst, 16);
        let m1 = fb.program().check_msg("first", Dir::To);
        let m2 = fb.program().check_msg("second", Dir::To);
        let (p, q) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
        for (dst, what) in [(p, m1), (q, m2)] {
            fb.emit(Inst::Pos {
                dst,
                dope,
                dim: 0,
                idx: x,
                what,
            });
        }
        let s = bin(fb, IntOp::Add, p, q);
        fb.emit(Inst::Ret { v: Some(s) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Pos { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn the_size_of_a_layout_of_known_length_is_a_constant() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let byte = pb.ty(Ty::Int { lo: 0, hi: 255 });
    let arr = pb.ty(Ty::Array {
        elem: byte,
        elem_desc: None,
        dims: 1,
    });
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Return, sp());
    let d = made(&mut fb, 1, 10, 0);
    let l = fb.reg(RegTy::Ptr);
    fb.emit(Inst::LayoutOf {
        dst: l,
        ty: arr,
        desc: d,
        cache: None,
    });
    let size = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: size,
        addr: l,
        w: Width::B8,
    });
    fb.emit(Inst::Ret { v: Some(size) });
    fb.finish();
    let p = pb.finish().expect("optimised code verifies");
    let code = &p.func(f).expect("f").code;
    assert_eq!(
        count(code, |i| matches!(i, Inst::LayoutOf { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(code, |i| matches!(i, Inst::Alloc { .. })),
        0,
        "{code:?}"
    );
    assert!(
        code.iter().any(|i| matches!(i, Inst::Const { v: 40, .. })),
        "{code:?}"
    );
}

/// A record of two integers and an array of them.
fn pair_types(pb: &mut ProgramBuilder) -> (TyId, TyId) {
    let int = pb.ty(Ty::Int { lo: 0, hi: 255 });
    let field = |name: &str| Field {
        name: name.into(),
        ty: int,
        desc: None,
    };
    let rec = pb.ty(Ty::Record {
        fields: vec![field("a"), field("b")],
    });
    let arr = pb.ty(Ty::Array {
        elem: rec,
        elem_desc: None,
        dims: 1,
    });
    (rec, arr)
}

#[test]
fn the_words_of_a_record_layout_and_the_stride_of_an_array_of_them_are_constants() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let (rec, arr) = pair_types(&mut pb);
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Return, sp());
    // Neither descriptor is known: the layout words that do not depend on it are.
    let ptr = |fb: &mut FuncBuilder<'_>, off: u32| {
        let a = fb.addr(AddrSpace::Inst, off);
        let d = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: d,
            addr: a,
            w: Width::B8,
        });
        d
    };
    let (d_rec, d_arr) = (ptr(&mut fb, 0), ptr(&mut fb, 8));
    let word = |fb: &mut FuncBuilder<'_>, ty: TyId, desc: Reg, off: i64| {
        let l = fb.reg(RegTy::Ptr);
        fb.emit(Inst::LayoutOf {
            dst: l,
            ty,
            desc,
            cache: None,
        });
        let at = fb.reg(RegTy::Ptr);
        let o = fb.int(off);
        fb.emit(Inst::Index {
            dst: at,
            base: l,
            pos: o,
            stride: 1,
        });
        let w = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: w,
            addr: at,
            w: Width::B8,
        });
        w
    };
    let size = word(&mut fb, rec, d_rec, 0);
    let second = word(&mut fb, rec, d_rec, 24);
    let stride = word(&mut fb, arr, d_arr, 16);
    let sum = bin(&mut fb, IntOp::Add, size, second);
    let sum = bin(&mut fb, IntOp::Add, sum, stride);
    fb.emit(Inst::Ret { v: Some(sum) });
    fb.finish();
    let p = pb.finish().expect("optimised code verifies");
    let code = &p.func(f).expect("f").code;
    let (l, _) = Ty::layout(&p.types, &p.consts, rec, &[]).expect("layout");
    assert_eq!((l.size, l.offsets.get(1).copied()), (16, Some(8)));
    // The size and the second field offset of the record, and the element size of the array:
    // 16 + 8 + 16 added up at compile time, the descriptors never read.
    assert_eq!(
        count(code, |i| matches!(
            i,
            Inst::LayoutOf { .. } | Inst::Load { .. }
        )),
        0,
        "{code:?}"
    );
    assert!(
        code.iter().any(|i| matches!(i, Inst::Const { v: 40, .. })),
        "{code:?}"
    );
}

/// A record variable copied from an `Alloc`ed block (made into frame memory by the
/// optimiser), then `between` and a load of its second field. Returns the code.
fn copied_record(between: impl FnOnce(&mut FuncBuilder<'_>, Reg, FuncId)) -> Vec<Inst> {
    let mut pb = ProgramBuilder::new();
    let g = pb.declare("g", FuncKind::Procedure, None);
    let mut gb = pb.define(g);
    gb.stmt(StmtKind::Return, sp());
    gb.emit(Inst::Ret { v: None });
    gb.finish();
    let (rec, _) = pair_types(&mut pb);
    let what = pb.check_msg("object", Dir::To);
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let off = fb.local("v", rec, 56, 8);
    fb.root(off + 16, RootKind::Handle);
    fb.stmt(StmtKind::Decl, sp());
    let n = fb.int(16);
    let src = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: src, size: n });
    let h = fb.reg(RegTy::Handle);
    fb.emit(Inst::New {
        dst: h,
        ty: rec,
        init: src,
        size: n,
        shared: false,
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
    between(&mut fb, src, g);
    let o = fb.int(8);
    let at = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: at,
        base: p,
        pos: o,
        stride: 1,
    });
    let b = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: b,
        addr: at,
        w: Width::B8,
    });
    fb.stmt(StmtKind::Return, sp());
    fb.emit(Inst::Ret { v: Some(b) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    p.func(f).expect("f").code.clone()
}

#[test]
fn a_copied_record_nothing_changes_is_read_from_its_source() {
    if !on() {
        return;
    }
    let frame = |i: &Inst| matches!(i, Inst::NewFrame { .. });
    // Nothing between the copy and the read: no copy.
    let code = copied_record(|_, _, _| {});
    assert_eq!(count(&code, frame), 0, "{code:?}");
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Load { .. })),
        1,
        "{code:?}"
    );
    // A store into the source, or a call, may change what the copy holds.
    let code = copied_record(|fb, src, _| {
        let v = fb.int(7);
        fb.emit(Inst::Store {
            addr: src,
            src: v,
            w: Width::B8,
        });
    });
    assert_eq!(count(&code, frame), 1, "{code:?}");
    let code = copied_record(|fb, _, g| {
        let args = fb.regs(&[]);
        fb.emit(Inst::Call {
            func: g,
            args,
            dst: None,
        });
    });
    assert_eq!(count(&code, frame), 1, "{code:?}");
}

type Via = Option<(StmtId, FuncId)>;

/// `f(a, b) = a + b` and a caller that passes two values it reads from the instance, or a
/// callee that lets the address of a parameter slot escape or branches: the caller's code and
/// its inlining record.
fn thin_call(variant: &str) -> (Vec<Inst>, Vec<Via>, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    fb.stmt(StmtKind::Return, sp());
    if variant == "escape" {
        // The address of a parameter's slot is stored: the slot is more than a way in.
        pin(&mut fb, sa);
    }
    let (x, y) = (load(&mut fb, sa), load(&mut fb, sb));
    let sum = bin(&mut fb, IntOp::Add, x, y);
    if variant == "branch" {
        let c = fb.reg(RegTy::I64);
        let zero = fb.int(0);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: sum,
            b: zero,
        });
        let (t, e) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t, f: e });
        fb.at(t);
        fb.emit(Inst::Ret { v: Some(zero) });
        fb.at(e);
    }
    fb.emit(Inst::Ret { v: Some(sum) });
    fb.finish();
    let g = pb.declare("g", FuncKind::Function, None);
    let mut gb = pb.define(g);
    gb.slot(16, 8);
    gb.stmt(StmtKind::Return, sp());
    let (u, v) = (unknown(&mut gb, 0), unknown(&mut gb, 8));
    let r = gb.reg(RegTy::I64);
    let args = gb.regs(&[u, v]);
    gb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(r),
    });
    gb.emit(Inst::Ret { v: Some(r) });
    gb.finish();
    let p = pb.finish().expect("optimised code verifies");
    let g = p.func(g).expect("g");
    let via = if g.via.is_empty() {
        Vec::new()
    } else {
        (0..g.code.len()).map(|pc| g.inlined_at(pc)).collect()
    };
    (g.code.clone(), via, f)
}

#[test]
fn a_call_of_a_thin_function_becomes_its_code() {
    if !on() {
        return;
    }
    let is_call = |i: &Inst| matches!(i, Inst::Call { .. });
    let (code, via, f) = thin_call("plain");
    assert_eq!(count(&code, is_call), 0, "{code:?}");
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { op: IntOp::Add, .. })),
        1,
        "{code:?}"
    );
    // The addition came from `f`: a trace lists `f` as a frame of its own.
    assert_eq!(via.len(), code.len());
    let at = code
        .iter()
        .position(|i| matches!(i, Inst::Bin { .. }))
        .expect("the addition");
    assert!(matches!(via[at], Some((_, c)) if c == f), "{via:?}");
    assert!(
        via[..at]
            .iter()
            .all(|v| v.is_none() || v.is_some_and(|(_, c)| c == f))
    );
    // A parameter slot whose address escapes: the call stays, nothing is recorded.
    let (code, via, _) = thin_call("escape");
    assert_eq!(count(&code, is_call), 1, "escape: {code:?}");
    assert!(via.is_empty(), "escape: {via:?}");
    // A branch: the callee's blocks are appended to the caller, which jumps to them and gets the
    // result back through a move and a jump to the rest of its block.
    let (code, via, f) = thin_call("branch");
    assert_eq!(count(&code, is_call), 0, "branch: {code:?}");
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        1,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Ret { .. })),
        1,
        "{code:?}"
    );
    assert_eq!(via.len(), code.len());
    let at = code
        .iter()
        .position(|i| matches!(i, Inst::CondBr { .. }))
        .expect("the branch");
    assert!(matches!(via[at], Some((_, c)) if c == f), "{via:?}");
}

#[test]
fn a_concatenation_keeps_its_block_and_what_it_read() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let (a, b) = (made(fb, 0, 1, 0), made(fb, 0, 1, 0));
        let n = fb.int(3);
        let m = fb.int(1);
        let one = fb.int(1);
        let two = fb.int(2);
        let size = fb.int(24);
        let data = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: data, size });
        let parts = fb.regs(&[a, two, x, one]);
        fb.emit(Inst::Cat {
            dst: data,
            n,
            m,
            parts,
            scalars: 0b10,
            bias: 0,
            logic: false,
            w: Width::B8,
        });
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: r,
            addr: data,
            w: Width::B8,
        });
        let _ = b;
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Cat { .. })),
        1,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        2,
        "{code:?}"
    );
    // The descriptor `b` nobody reads goes with its allocation; `a` is read by the `Cat`.
    assert_eq!(
        count(&code, |i| matches!(i, Inst::MakeDope { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn a_select_on_a_constant_or_of_equal_values_is_a_move() {
    if !on() {
        return;
    }
    let sel = |fb: &mut FuncBuilder<'_>, c: Reg, a: Reg, b: Reg| {
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Select { dst, c, a, b });
        dst
    };
    let code = build(FuncKind::Function, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        pin(fb, 16);
        let (x, y, c) = (load(fb, 0), load(fb, 8), load(fb, 16));
        let one = fb.int(1);
        let zero = fb.int(0);
        let a = sel(fb, one, x, y);
        let b = sel(fb, zero, x, y);
        let same = sel(fb, c, x, x);
        let real = sel(fb, c, a, b);
        let r = bin(fb, IntOp::Add, same, real);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Select { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn a_select_nobody_reads_goes() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        pin(fb, 16);
        let (x, y, c) = (load(fb, 0), load(fb, 8), load(fb, 16));
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Select { dst, c, a: x, b: y });
        fb.emit(Inst::Ret { v: Some(x) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Select { .. })),
        0,
        "{code:?}"
    );
}

fn unknown(fb: &mut FuncBuilder<'_>, off: u32) -> Reg {
    let a = fb.addr(AddrSpace::Inst, off);
    let d = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: d,
        addr: a,
        w: Width::B8,
    });
    d
}

#[test]
fn a_slice_emptiness_test_compares_the_bounds() {
    if !on() {
        return;
    }
    for (dir, want_a, want_b) in [(0, 1, 0), (1, 0, 1)] {
        let code = build(FuncKind::Function, |fb| {
            fb.slot(16, 8);
            fb.stmt(StmtKind::Return, sp());
            // (Only code in a loop is cleaned up: the test goes round once more.)
            let head = fb.block();
            fb.emit(Inst::Br { to: head });
            fb.at(head);
            let (l, r) = (unknown(fb, 0), unknown(fb, 8));
            let at = fb.reg(RegTy::Ptr);
            let n24 = fb.int(24);
            fb.emit(Inst::Alloc { dst: at, size: n24 });
            let d = fb.int(dir);
            let parts = fb.regs(&[l, r, d]);
            fb.emit(Inst::MakeDope { at, parts });
            let len = fb.reg(RegTy::I64);
            fb.emit(Inst::DopeLen {
                dst: len,
                dope: at,
                dim: 0,
            });
            let z = fb.int(0);
            let c = fb.reg(RegTy::I64);
            fb.emit(Inst::Cmp {
                op: CmpOp::Eq,
                dst: c,
                a: len,
                b: z,
            });
            let (t, e) = (fb.block(), fb.block());
            fb.emit(Inst::CondBr { c, t, f: e });
            fb.at(t);
            fb.emit(Inst::Ret { v: Some(l) });
            fb.at(e);
            let done = fb.block();
            fb.emit(Inst::CondBr {
                c: r,
                t: head,
                f: done,
            });
            fb.at(done);
            fb.emit(Inst::Ret { v: Some(r) });
        });
        for gone in [
            count(&code, |i| matches!(i, Inst::DopeLen { .. })),
            count(&code, |i| matches!(i, Inst::MakeDope { .. })),
            count(&code, |i| matches!(i, Inst::Alloc { .. })),
        ] {
            assert_eq!(gone, 0, "{code:?}");
        }
        // Empty when the bounds are the wrong way round: loads come in order l, r.
        let loads: Vec<Reg> = code
            .iter()
            .filter_map(|i| match i {
                Inst::Load { dst, .. } => Some(*dst),
                _ => None,
            })
            .collect();
        let cmp = code
            .iter()
            .find_map(|i| match i {
                Inst::Cmp { op, a, b, .. } => Some((*op, *a, *b)),
                _ => None,
            })
            .expect("one comparison");
        assert_eq!(
            cmp,
            (CmpOp::Lt, loads[want_a], loads[want_b]),
            "direction {dir}: {code:?}"
        );
    }
}

#[test]
fn a_flag_set_on_each_path_is_branched_on_there() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        // (Only code in a loop is cleaned up: the test goes round once more.)
        let head = fb.block();
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let (x, y, z) = (unknown(fb, 0), unknown(fb, 8), unknown(fb, 16));
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: x,
            b: y,
        });
        let (bt, bf, join, yes, no) = (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t: bt, f: bf });
        // `flag := x = z` on one path, `flag := false` on the other.
        let flag = fb.reg(RegTy::I64);
        fb.at(bt);
        let c2 = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c2,
            a: x,
            b: z,
        });
        fb.emit(Inst::Move { dst: flag, src: c2 });
        fb.emit(Inst::Br { to: join });
        fb.at(bf);
        fb.emit(Inst::Const { dst: flag, v: 0 });
        fb.emit(Inst::Br { to: join });
        fb.at(join);
        fb.emit(Inst::CondBr {
            c: flag,
            t: yes,
            f: no,
        });
        fb.at(yes);
        fb.emit(Inst::Ret { v: Some(x) });
        fb.at(no);
        let done = fb.block();
        fb.emit(Inst::CondBr {
            c: y,
            t: head,
            f: done,
        });
        fb.at(done);
        fb.emit(Inst::Ret { v: Some(y) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Move { .. })),
        0,
        "{code:?}"
    );
    // The two tests are branches, and the loop's exit test follows each path that reaches it
    // (the flag is gone, and the false path goes straight to that test).
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        4,
        "{code:?}"
    );
    let cmp_eq = code
        .iter()
        .find_map(|i| match i {
            Inst::Cmp {
                op: CmpOp::Eq, dst, ..
            } => Some(*dst),
            _ => None,
        })
        .expect("the equality");
    assert!(
        code.iter()
            .any(|i| matches!(i, Inst::CondBr { c, .. } if *c == cmp_eq)),
        "{code:?}"
    );
}

#[test]
fn a_constant_of_a_multiply_defined_register_is_known_until_it_is_written_again() {
    if !on() {
        return;
    }
    // r is written in three blocks, so it is no single-definition constant; in b2 it holds 0
    // from its `Const` to the branch, and the branch on its negation is decided.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let zero = fb.int(0);
        let r = fb.reg(RegTy::I64);
        let (c, n) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
        fb.emit(Inst::Const { dst: r, v: 1 });
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: x,
            b: zero,
        });
        let (b2, b3, yes, no) = (fb.block(), fb.block(), fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t: b2, f: b3 });
        fb.at(b2);
        fb.emit(Inst::Const { dst: r, v: 0 });
        fb.emit(Inst::Bool {
            op: BoolOp::Not,
            dst: n,
            a: r,
            b: r,
        });
        fb.emit(Inst::CondBr {
            c: n,
            t: yes,
            f: no,
        });
        fb.at(b3);
        fb.emit(Inst::Const { dst: r, v: 7 });
        fb.emit(Inst::Br { to: no });
        fb.at(yes);
        let one = fb.int(11);
        fb.emit(Inst::Ret { v: Some(one) });
        fb.at(no);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::CondBr { .. })),
        1,
        "{code:?}"
    );
}

#[test]
fn loads_of_instance_data_survive_statements_and_stores_through_pointers() {
    if !on() {
        return;
    }
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let fa = fb.addr(AddrSpace::Frame, 0);
        let ptr = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: ptr,
            addr: fa,
            w: Width::B8,
        });
        let ia = fb.addr(AddrSpace::Inst, 8);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: x,
            addr: ia,
            w: Width::B8,
        });
        fb.emit(Inst::Stmt);
        fb.emit(Inst::Store {
            addr: ptr,
            src: x,
            w: Width::B8,
        });
        let ib = fb.addr(AddrSpace::Inst, 8);
        let y = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: y,
            addr: ib,
            w: Width::B8,
        });
        let s = bin(fb, IntOp::Add, x, y);
        fb.emit(Inst::Ret { v: Some(s) });
    });
    let inst_loads = count(
        &code,
        |i| matches!(i, Inst::Load { addr, .. } if code.iter().any(|j| matches!(j, Inst::AddrOf { dst, space: AddrSpace::Inst, .. } if dst == addr))),
    );
    assert_eq!(inst_loads, 1, "{code:?}");
}

#[test]
fn a_multiply_defined_register_is_shared_until_it_is_written_again() {
    if !on() {
        return;
    }
    // i is written twice: the two sums with the first value are one, the one after the second
    // write is another.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let x = load(fb, 0);
        let i = fb.reg(RegTy::I64);
        fb.emit(Inst::Move { dst: i, src: x });
        let k = fb.int(5);
        let a = bin(fb, IntOp::Add, i, k);
        let b = bin(fb, IntOp::Add, i, k);
        let y = load(fb, 8);
        fb.emit(Inst::Move { dst: i, src: y });
        let c = bin(fb, IntOp::Add, i, k);
        let ab = bin(fb, IntOp::Mul, a, b);
        let r = bin(fb, IntOp::Mul, ab, c);
        fb.emit(Inst::Ret { v: Some(r) });
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Bin { op: IntOp::Add, .. })),
        2,
        "{code:?}"
    );
}

/// `f(a, b) = a + b`, `g(a) = f(a, 1)` and `h` calling `g` on a value of the instance.
#[test]
fn a_chain_of_inlined_calls_is_recorded_whole() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    fb.stmt(StmtKind::Return, sp());
    let (x, y) = (load(&mut fb, sa), load(&mut fb, sb));
    let sum = bin(&mut fb, IntOp::Add, x, y);
    fb.emit(Inst::Ret { v: Some(sum) });
    fb.finish();
    let g = pb.declare("g", FuncKind::Function, None);
    let mut gb = pb.define(g);
    let ga = gb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    gb.stmt(StmtKind::Return, sp());
    let a = load(&mut gb, ga);
    let one = gb.int(1);
    let r = gb.reg(RegTy::I64);
    let args = gb.regs(&[a, one]);
    gb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(r),
    });
    gb.emit(Inst::Ret { v: Some(r) });
    gb.finish();
    let h = pb.declare("h", FuncKind::Function, None);
    let mut hb = pb.define(h);
    hb.slot(16, 8);
    hb.stmt(StmtKind::Return, sp());
    let u = unknown(&mut hb, 0);
    let r = hb.reg(RegTy::I64);
    let args = hb.regs(&[u]);
    hb.emit(Inst::Call {
        func: g,
        args,
        dst: Some(r),
    });
    hb.emit(Inst::Ret { v: Some(r) });
    hb.finish();
    let p = pb.finish().expect("optimised code verifies");
    let hf = p.func(h).expect("h");
    assert_eq!(count(&hf.code, |i| matches!(i, Inst::Call { .. })), 0);
    let at = hf
        .code
        .iter()
        .position(|i| matches!(i, Inst::Bin { op: IntOp::Add, .. }))
        .expect("the addition");
    let chain: Vec<FuncId> = hf.inlined_chain(at).map(|n| n.callee).collect();
    assert_eq!(chain, vec![f, g], "{:?}", hf.via_nodes);
}

#[test]
fn a_case_of_the_callee_becomes_compares() {
    if !on() {
        return;
    }
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    fb.stmt(StmtKind::Return, sp());
    let x = load(&mut fb, sa);
    let (b1, b2, bd) = (fb.block(), fb.block(), fb.block());
    let table = fb.program().case_table(CaseTable {
        arms: vec![(1, 1, b1), (5, 7, b2)],
        default: bd,
    });
    fb.emit(Inst::Case { v: x, table });
    fb.at(b1);
    let ten = fb.int(10);
    fb.emit(Inst::Ret { v: Some(ten) });
    fb.at(b2);
    let twenty = fb.int(20);
    fb.emit(Inst::Ret { v: Some(twenty) });
    fb.at(bd);
    let zero = fb.int(0);
    fb.emit(Inst::Ret { v: Some(zero) });
    fb.finish();
    let g = pb.declare("g", FuncKind::Function, None);
    let mut gb = pb.define(g);
    gb.slot(16, 8);
    gb.stmt(StmtKind::Return, sp());
    let u = unknown(&mut gb, 0);
    let r = gb.reg(RegTy::I64);
    let args = gb.regs(&[u]);
    gb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(r),
    });
    gb.emit(Inst::Ret { v: Some(r) });
    gb.finish();
    let p = pb.finish().expect("optimised code verifies");
    let code = &p.func(g).expect("g").code;
    assert_eq!(
        count(code, |i| matches!(i, Inst::Call { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(code, |i| matches!(i, Inst::Case { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(code, |i| matches!(i, Inst::CondBr { .. })),
        2,
        "{code:?}"
    );
}

#[test]
fn a_byte_variable_lives_in_a_register() {
    if !on() {
        return;
    }
    // A boolean or enumeration variable has a one-byte slot: only direct accesses of that
    // width, so it becomes a register like an integer.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let flag = fb.addr(AddrSpace::Frame, 8);
        let one = fb.int(1);
        fb.emit(Inst::Store {
            addr: flag,
            src: one,
            w: Width::B1,
        });
        let x = load(fb, 0);
        let zero = fb.int(0);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: x,
            b: zero,
        });
        let (t, e) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t, f: e });
        fb.at(t);
        fb.emit(Inst::Store {
            addr: flag,
            src: zero,
            w: Width::B1,
        });
        fb.emit(Inst::Br { to: e });
        fb.at(e);
        let y = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: y,
            addr: flag,
            w: Width::B1,
        });
        fb.emit(Inst::Ret { v: Some(y) });
    });
    assert_eq!(
        count(&code, |i| matches!(
            i,
            Inst::Load { w: Width::B1, .. } | Inst::Store { w: Width::B1, .. }
        )),
        0,
        "{code:?}"
    );
}

/// A fat pair made of temporaries and constants is no GC root; one holding a pointer of unknown
/// origin (it may point into a heap) keeps its `TempRoot`.
#[test]
fn a_fat_pair_of_temporaries_needs_no_root() {
    if !on() {
        return;
    }
    let roots = |unknown: bool| {
        let code = build(FuncKind::Function, |fb| {
            let p = fb.slot(8, 8);
            fb.stmt(StmtKind::Return, sp());
            let (eight, sixteen) = (fb.int(8), fb.int(16));
            let data = fb.reg(RegTy::Ptr);
            if unknown {
                let a = fb.addr(AddrSpace::Frame, p);
                fb.emit(Inst::Load {
                    dst: data,
                    addr: a,
                    w: Width::B8,
                });
            } else {
                fb.emit(Inst::Alloc {
                    dst: data,
                    size: eight,
                });
            }
            let dope = fb.addr(AddrSpace::Inst, 0);
            let fat = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Alloc {
                dst: fat,
                size: sixteen,
            });
            fb.emit(Inst::Store {
                addr: fat,
                src: data,
                w: Width::B8,
            });
            let at8 = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: at8,
                base: fat,
                pos: eight,
                stride: 1,
            });
            fb.emit(Inst::Store {
                addr: at8,
                src: dope,
                w: Width::B8,
            });
            fb.emit(Inst::TempRoot {
                addr: fat,
                desc: None,
                ty: None,
            });
            fb.emit(Inst::Ret { v: Some(sixteen) });
        });
        count(&code, |i| matches!(i, Inst::TempRoot { .. }))
    };
    assert_eq!(roots(false), 0);
    assert_eq!(roots(true), 1);
}

/// A function of two parameters that makes a descriptor `(l, r, 1)` in temporary memory,
/// optionally writes a word of it, and loads its three words; the loads left.
fn dope_loads_left(between: impl FnOnce(&mut FuncBuilder<'_>, Reg)) -> usize {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sl = fb.param("l", Width::B8, ParamClass::Value, Mode::In, None);
    let sr = fb.param("r", Width::B8, ParamClass::Value, Mode::In, None);
    fb.stmt(StmtKind::Return, sp());
    let (l, r) = (load(&mut fb, sl), load(&mut fb, sr));
    let one = fb.int(1);
    let size = fb.int(24);
    let dope = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: dope, size });
    let parts = fb.regs(&[l, r, one]);
    fb.emit(Inst::MakeDope { at: dope, parts });
    between(&mut fb, dope);
    let mut sum = fb.int(0);
    for off in [0, 8, 16] {
        let at = fb.int(off);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: a,
            base: dope,
            pos: at,
            stride: 1,
        });
        let w = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: w,
            addr: a,
            w: Width::B8,
        });
        sum = bin(&mut fb, IntOp::Add, sum, w);
    }
    fb.emit(Inst::Ret { v: Some(sum) });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("f").code;
    count(code, |i| matches!(i, Inst::Load { w: Width::B8, .. })) - 2
}

#[test]
fn a_made_descriptor_gives_its_bounds_back() {
    if !on() {
        return;
    }
    // (Two of the loads are the parameters.) The three words are the registers it was made from.
    assert_eq!(dope_loads_left(|_, _| {}), 0);
    // A statement marker keeps what is known of a block of this function.
    assert_eq!(
        dope_loads_left(|fb, _| {
            fb.stmt(StmtKind::Null, sp());
        }),
        0
    );
    // A store into the descriptor at a constant offset changes the word it writes only: the
    // other two still read back the registers, the stored word what was stored.
    assert_eq!(
        dope_loads_left(|fb, dope| {
            let at = fb.int(8);
            let a = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: a,
                base: dope,
                pos: at,
                stride: 1,
            });
            let v = fb.int(77);
            fb.emit(Inst::Store {
                addr: a,
                src: v,
                w: Width::B8,
            });
        }),
        0
    );
    // At an offset only the run knows, the optimiser cannot tell which word: the loads stay.
    assert!(
        dope_loads_left(|fb, dope| {
            let at = load(fb, 0);
            let a = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: a,
                base: dope,
                pos: at,
                stride: 1,
            });
            let v = fb.int(77);
            fb.emit(Inst::Store {
                addr: a,
                src: v,
                w: Width::B8,
            });
        }) > 0
    );
}

#[test]
fn a_branch_whose_arms_lead_to_the_same_block_is_a_jump() {
    if !on() {
        return;
    }
    // The null-range test of a local array: `if 0 <= n - 1` jumps to a block that only jumps on.
    let code = build(FuncKind::Function, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        let x = unknown(fb, 0);
        let y = unknown(fb, 8);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Le,
            dst: c,
            a: x,
            b: y,
        });
        let (b1, b2) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t: b1, f: b2 });
        fb.at(b1);
        fb.emit(Inst::Br { to: b2 });
        fb.at(b2);
        let s = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: s,
            a: x,
            b: y,
        });
        let t = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Mul,
            dst: t,
            a: s,
            b: y,
        });
        fb.emit(Inst::Ret { v: Some(t) });
    });
    assert_eq!(
        count(&code, |i| matches!(
            i,
            Inst::CondBr { .. } | Inst::Cmp { .. }
        )),
        0,
        "{code:?}"
    );
}

/// `check_msg`, a value of unknown `x`: the bounds of `v(8 * (x + 1) - 1 downto 8 * x)` (the
/// factor of the right bound `mul_r`) in a fresh descriptor, checked against a descriptor of
/// length 8. `x` is defined twice, as a loop index is.
fn scaled_slice_checks(mul_r: i64) -> usize {
    let code = build(FuncKind::Procedure, |fb| {
        fb.slot(32, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let (i, j) = (load(fb, 0), load(fb, 8));
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::Move { dst: x, src: i });
        fb.emit(Inst::Move { dst: x, src: j });
        let what = fb.program().check_msg("range", Dir::To);
        let (lo, hi) = (fb.int(-1000), fb.int(1000));
        let (eight, one, dir) = (fb.int(8), fb.int(1), fb.int(1));
        let rb = fb.int(mul_r);
        let br = |fb: &mut FuncBuilder<'_>, op, a, b| {
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what,
            });
            dst
        };
        let t = br(fb, IntOp::Add, x, one);
        let m = br(fb, IntOp::Mul, eight, t);
        let left = bin(fb, IntOp::Sub, m, one);
        let right = br(fb, IntOp::Mul, rb, x);
        let at = fb.reg(RegTy::Ptr);
        let n = fb.int(24);
        fb.emit(Inst::Alloc { dst: at, size: n });
        let parts = fb.regs(&[left, right, dir]);
        fb.emit(Inst::MakeDope { at, parts });
        let have = made(fb, 7, 0, 1);
        let what = fb.program().check_msg("length", Dir::To);
        fb.emit(Inst::SubtypeCheck {
            dope: have,
            want: at,
            dims: 1,
            what,
        });
        fb.emit(Inst::Ret { v: None });
    });
    count(&code, |i| matches!(i, Inst::SubtypeCheck { .. }))
}

#[test]
fn a_slice_of_a_scaled_index_has_a_known_length() {
    if !on() {
        return;
    }
    // 8 * (x + 1) - 1 downto 8 * x is 8 elements whatever x is; with another factor it is not.
    assert_eq!(scaled_slice_checks(8), 0);
    assert_eq!(scaled_slice_checks(7), 1);
}

/// A native `to_unsigned (v, size)` whose descriptor is checked against one of length 8, with a
/// descriptor made in between.
fn native_result_checks(size: i64) -> usize {
    native_checks(
        "yavs.ieee.numeric_std.to_unsigned(integer,integer)unsigned",
        &[None, Some(size)],
    )
}

/// The same for any native: each argument register is unknown (`None`) or the constant.
fn native_checks(symbol: &str, args: &[Option<i64>]) -> usize {
    let code = build(FuncKind::Procedure, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let v = load(fb, 0);
        let native = fb.program().native(symbol, None);
        let regs: Vec<Reg> = args.iter().map(|a| a.map_or(v, |c| fb.int(c))).collect();
        let args = fb.regs(&regs);
        let res = fb.reg(RegTy::Ptr);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: Some(res),
        });
        let eight = fb.int(8);
        let slot = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: slot,
            base: res,
            pos: eight,
            stride: 1,
        });
        let dope = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: dope,
            addr: slot,
            w: Width::B8,
        });
        let want = made(fb, 7, 0, 1);
        let what = fb.program().check_msg("length", Dir::To);
        fb.emit(Inst::SubtypeCheck {
            dope,
            want,
            dims: 1,
            what,
        });
        fb.emit(Inst::Ret { v: None });
    });
    count(&code, |i| matches!(i, Inst::SubtypeCheck { .. }))
}

#[test]
fn the_length_of_a_native_result_is_known() {
    if !on() {
        return;
    }
    assert_eq!(native_result_checks(8), 0);
    assert_eq!(native_result_checks(7), 1);
}

#[test]
fn a_null_operand_gives_a_null_sum() {
    if !on() {
        return;
    }
    let add = "yavs.ieee.numeric_std.\"+\"(unsigned,unsigned)unsigned";
    // (6 downto 0) + (7 downto 0) is 8 elements; with a null operand the sum is null.
    let sum = |l: [i64; 3]| {
        native_checks(
            add,
            &[
                None,
                Some(l[0]),
                Some(l[1]),
                Some(l[2]),
                None,
                Some(7),
                Some(0),
                Some(1),
            ],
        )
    };
    assert_eq!(sum([6, 0, 1]), 0);
    assert_eq!(sum([-1, 0, 1]), 1);
}

/// `to_unsigned (v, size)` whose result bounds, read from its descriptor, are the bounds of the
/// first operand of an addition of an operand of `other` elements: the length of the sum
/// checked against 8. `same` reads the three bounds from the one descriptor, else the right
/// bound from the descriptor of a second `to_unsigned`.
fn chained_native_checks(size: i64, other: i64, same: bool) -> usize {
    let code = build(FuncKind::Procedure, |fb| {
        fb.slot(16, 8);
        fb.stmt(StmtKind::Return, sp());
        pin(fb, 0);
        let v = load(fb, 0);
        let to_u = fb.program().native(
            "yavs.ieee.numeric_std.to_unsigned(integer,integer)unsigned",
            None,
        );
        let add = fb.program().native(
            "yavs.ieee.numeric_std.\"+\"(unsigned,unsigned)unsigned",
            None,
        );
        let result = |fb: &mut FuncBuilder<'_>, size: i64| {
            let n = fb.int(size);
            let args = fb.regs(&[v, n]);
            let res = fb.reg(RegTy::Ptr);
            fb.emit(Inst::CallNative {
                native: to_u,
                args,
                dst: Some(res),
            });
            let data = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Load {
                dst: data,
                addr: res,
                w: Width::B8,
            });
            let eight = fb.int(8);
            let slot = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: slot,
                base: res,
                pos: eight,
                stride: 1,
            });
            let dope = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Load {
                dst: dope,
                addr: slot,
                w: Width::B8,
            });
            (data, dope)
        };
        let word = |fb: &mut FuncBuilder<'_>, dope: Reg, off: i64| {
            let k = fb.int(off);
            let a = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: a,
                base: dope,
                pos: k,
                stride: 1,
            });
            let w = fb.reg(RegTy::I64);
            fb.emit(Inst::Load {
                dst: w,
                addr: a,
                w: Width::B8,
            });
            w
        };
        let (data, dope) = result(fb, size);
        let (_, dope2) = if same { (data, dope) } else { result(fb, size) };
        let left = {
            let w = fb.reg(RegTy::I64);
            fb.emit(Inst::Load {
                dst: w,
                addr: dope,
                w: Width::B8,
            });
            w
        };
        let right = word(fb, dope2, 8);
        let dir = word(fb, dope, 16);
        let (l2, r2, d2) = (fb.int(other - 1), fb.int(0), fb.int(1));
        let regs = [data, left, right, dir, data, l2, r2, d2];
        let args = fb.regs(&regs);
        let sum = fb.reg(RegTy::Ptr);
        fb.emit(Inst::CallNative {
            native: add,
            args,
            dst: Some(sum),
        });
        let eight = fb.int(8);
        let slot = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: slot,
            base: sum,
            pos: eight,
            stride: 1,
        });
        let have = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: have,
            addr: slot,
            w: Width::B8,
        });
        let want = made(fb, 7, 0, 1);
        let what = fb.program().check_msg("length", Dir::To);
        fb.emit(Inst::SubtypeCheck {
            dope: have,
            want,
            dims: 1,
            what,
        });
        fb.emit(Inst::Ret { v: None });
    });
    count(&code, |i| matches!(i, Inst::SubtypeCheck { .. }))
}

#[test]
fn bounds_read_from_a_native_result_give_the_length_of_the_next_native() {
    if !on() {
        return;
    }
    // 8 elements plus 3: 8. 5 plus 8: 8. A bound from another descriptor, or 9 elements: stays.
    assert_eq!(chained_native_checks(8, 3, true), 0);
    assert_eq!(chained_native_checks(5, 8, true), 0);
    assert_eq!(chained_native_checks(9, 3, true), 1);
    assert_eq!(chained_native_checks(8, 3, false), 1);
}
