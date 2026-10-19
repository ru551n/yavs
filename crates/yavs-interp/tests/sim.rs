//! Hand-written IR run through the real kernel at 1 and 4 threads with identical traces.

mod common;

use common::*;
use yavs_ffi::{CType, ForeignFn};
use yavs_interp::{Bindings, Limits, Runtime};
use yavs_ir::*;
use yavs_kernel::*;

fn byte_sig(name: &'static str, n: u32, es: u32) -> (&'static str, SignalType, Value) {
    let ty = SignalType::bytes(es, n);
    (name, ty, Value::Bytes(vec![0; (es * n) as usize]))
}

/// `clk <= not clk after 5 ns` and a counter of rising edges that reports and stops at 4.
#[test]
fn clock_and_counter() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let stop = pb.native("yavs.env.stop.integer", None);
    let clkgen = pb.declare("clkgen", FuncKind::Process, None);
    let counter = pb.declare("counter", FuncKind::Process, None);
    {
        let mut fb = pb.define(clkgen);
        stmt(&mut fb, StmtKind::SigAssign);
        let v = read_scalar(&mut fb, 0);
        let one = fb.int(1);
        let nv = bin(&mut fb, IntOp::Sub, one, v);
        drive(&mut fb, 0, 5 * NS, nv, Width::B1);
        wait_on_loop(&mut fb, 0, BlockId(0));
        fb.finish();
    }
    {
        let mut fb = pb.define(counter);
        let count = fb.local("count", int, 8, 8);
        let (b1, b2, b3) = (fb.block(), fb.block(), fb.block());
        stmt(&mut fb, StmtKind::If);
        let c = read_scalar(&mut fb, 0);
        let one = fb.int(1);
        let e = cmp(&mut fb, CmpOp::Eq, c, one);
        fb.emit(Inst::CondBr { c: e, t: b1, f: b2 });
        fb.at(b1);
        stmt(&mut fb, StmtKind::VarAssign);
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        let one = fb.int(1);
        let v1 = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Frame, count, v1, Width::B8);
        stmt(&mut fb, StmtKind::SigAssign);
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        drive(&mut fb, 0, 0, v, Width::B8);
        stmt(&mut fb, StmtKind::If);
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        let four = fb.int(4);
        let e = cmp(&mut fb, CmpOp::Eq, v, four);
        fb.emit(Inst::CondBr { c: e, t: b3, f: b2 });
        fb.at(b3);
        stmt(&mut fb, StmtKind::Report);
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        report_image(&mut fb, "count = ", int, v);
        stmt(&mut fb, StmtKind::Call);
        let zero = fb.int(0);
        let args = fb.regs(&[zero]);
        fb.emit(Inst::CallNative {
            native: stop,
            args,
            dst: None,
        });
        fb.emit(Inst::Br { to: b2 });
        fb.at(b2);
        wait_on_loop(&mut fb, 0, BlockId(0));
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.signals = vec![byte_sig("clk", 1, 1), byte_sig("cnt", 1, 8)];
    d.procs = vec![
        ProcSpec {
            name: "clkgen",
            func: clkgen,
            drives: vec![(0, 1)],
        },
        ProcSpec {
            name: "counter",
            func: counter,
            drives: vec![(1, 1)],
        },
    ];
    let (lines, r) = d.run(Time::ns(1000));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Stop(0))));
    assert!(has(&lines, "35 ns +0 p1 Note: count = 4"), "{lines:#?}");
    // stop ends the simulation after the delta, so cnt never shows 4.
    assert!(
        has(&lines, "25 ns +1 cnt=[3, 0, 0, 0, 0, 0, 0, 0]"),
        "{lines:#?}"
    );
    assert!(!has(&lines, "cnt=[4"), "{lines:#?}");
    assert!(has(&lines, "5 ns +0 clk=[1]"), "{lines:#?}");
    assert!(!has(&lines, "40 ns"), "{lines:#?}");
}

/// a -> b -> c, one delta each; processes see the delta number.
#[test]
fn delta_ordering() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let p1 = pb.declare("p1", FuncKind::Process, None);
    {
        let mut fb = pb.define(p1);
        stmt(&mut fb, StmtKind::SigAssign);
        let one = fb.int(1);
        drive(&mut fb, 0, 0, one, Width::B8);
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut follow = |name: &str, src: u32, dst_slot: u32| {
        let f = pb.declare(name, FuncKind::Process, None);
        let mut fb = pb.define(f);
        stmt(&mut fb, StmtKind::Report);
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Delta { dst: d });
        report_image(&mut fb, &format!("{name} at delta "), int, d);
        stmt(&mut fb, StmtKind::SigAssign);
        let x = read_scalar(&mut fb, src);
        let one = fb.int(1);
        let y = bin(&mut fb, IntOp::Add, x, one);
        drive(&mut fb, dst_slot, 0, y, Width::B8);
        wait_on_loop(&mut fb, src, BlockId(0));
        fb.finish();
        f
    };
    let p2 = follow("p2", 0, 0);
    let p3 = follow("p3", 1, 0);
    let mut d = Design::new(pb);
    d.signals = vec![
        byte_sig("a", 1, 8),
        byte_sig("b", 1, 8),
        byte_sig("c", 1, 8),
    ];
    d.procs = vec![
        ProcSpec {
            name: "p1",
            func: p1,
            drives: vec![(0, 1)],
        },
        ProcSpec {
            name: "p2",
            func: p2,
            drives: vec![(1, 1)],
        },
        ProcSpec {
            name: "p3",
            func: p3,
            drives: vec![(2, 1)],
        },
    ];
    let (lines, r) = d.run(Time::ns(10));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    let want = [
        "0 fs +0 p1 Note: p2 at delta 0",
        "0 fs +0 p2 Note: p3 at delta 0",
        "0 fs +1 a=[1, 0, 0, 0, 0, 0, 0, 0]",
        "0 fs +1 p1 Note: p2 at delta 1",
        "0 fs +2 b=[2, 0, 0, 0, 0, 0, 0, 0]",
        "0 fs +3 c=[3, 0, 0, 0, 0, 0, 0, 0]",
    ];
    let mut at = 0;
    for w in want {
        let i = lines[at..]
            .iter()
            .position(|l| l == w)
            .unwrap_or_else(|| panic!("missing or out of order: {w}\n{lines:#?}"));
        at += i + 1;
    }
}

/// `wait for`, a `wait on` inside a procedure (resumable frames), `wait on .. until .. for`
/// that wakes on the condition and one that times out.
#[test]
fn wait_forms() {
    let mut pb = ProgramBuilder::new();
    let time = time_ty(&mut pb);
    let driver = pb.declare("driver", FuncKind::Process, None);
    let tick = pb.declare("tick", FuncKind::Procedure, None);
    let w = pb.declare("w", FuncKind::Process, None);
    {
        let mut fb = pb.define(driver);
        stmt(&mut fb, StmtKind::SigAssign);
        let size = fb.int(32);
        let wave = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: wave, size });
        for (k, (t, v)) in [(15 * NS, 1), (30 * NS, 2)].into_iter().enumerate() {
            let off = fb.int(16 * k as i64);
            let at = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: at,
                base: wave,
                pos: off,
                stride: 1,
            });
            let tr = fb.int(t);
            fb.emit(Inst::Store {
                addr: at,
                src: tr,
                w: Width::B8,
            });
            let eight = fb.int(8);
            let va = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: va,
                base: at,
                pos: eight,
                stride: 1,
            });
            let vr = fb.int(v);
            fb.emit(Inst::Store {
                addr: va,
                src: vr,
                w: Width::B8,
            });
        }
        let n = fb.int(2);
        fb.emit(Inst::Drive {
            driver: 0,
            target: None,
            wave,
            n,
            mode: DriveMode::Transport,
            reject: None,
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    {
        let mut fb = pb.define(tick);
        let x = fb.param("x", Width::Sig, ParamClass::Signal, Mode::In, None);
        stmt(&mut fb, StmtKind::Wait);
        let a = fb.addr(AddrSpace::Frame, x);
        let s = fb.reg(RegTy::Sig);
        fb.emit(Inst::Load {
            dst: s,
            addr: a,
            w: Width::Sig,
        });
        fb.emit(Inst::WaitOn { sig: s });
        fb.emit(Inst::Suspend { resume: 0 });
        stmt(&mut fb, StmtKind::Report);
        report(&mut fb, "in procedure");
        wait_for(&mut fb, 3 * NS);
        stmt(&mut fb, StmtKind::Return);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    {
        let mut fb = pb.define(w);
        let dl = fb.slot(8, 8);
        wait_for(&mut fb, 10 * NS);
        stmt(&mut fb, StmtKind::Report);
        report(&mut fb, "t1");
        stmt(&mut fb, StmtKind::Call);
        let s = sig(&mut fb, 0, 0, 1);
        let args = fb.regs(&[s]);
        fb.emit(Inst::Call {
            func: tick,
            args,
            dst: None,
        });
        // wait on s until s = want for limit; then report the time with `msg`.
        let wait_until = |fb: &mut FuncBuilder<'_>, want: i64, limit: i64, msg: &str| {
            stmt(fb, StmtKind::Wait);
            let now = fb.reg(RegTy::I64);
            fb.emit(Inst::Now { dst: now });
            let t = fb.int(limit);
            let deadline = bin(fb, IntOp::Add, now, t);
            store(fb, AddrSpace::Frame, dl, deadline, Width::B8);
            let (l, chk, exit) = (fb.block(), fb.block(), fb.block());
            fb.emit(Inst::Br { to: l });
            fb.at(l);
            let s = sig(fb, 0, 0, 1);
            fb.emit(Inst::WaitOn { sig: s });
            let d = load(fb, AddrSpace::Frame, dl, Width::B8);
            fb.emit(Inst::WaitAt { t: d });
            fb.emit(Inst::Suspend { resume: 0 });
            let to = fb.reg(RegTy::I64);
            fb.emit(Inst::TimedOut { dst: to });
            fb.emit(Inst::CondBr {
                c: to,
                t: exit,
                f: chk,
            });
            fb.at(chk);
            let v = read_scalar(fb, 0);
            let wv = fb.int(want);
            let e = cmp(fb, CmpOp::Eq, v, wv);
            fb.emit(Inst::CondBr {
                c: e,
                t: exit,
                f: l,
            });
            fb.at(exit);
            stmt(fb, StmtKind::Report);
            let now = fb.reg(RegTy::I64);
            fb.emit(Inst::Now { dst: now });
            report_image(fb, msg, time, now);
        };
        wait_until(&mut fb, 2, 100 * NS, "woke at ");
        wait_until(&mut fb, 5, 20 * NS, "timed out at ");
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.signals = vec![byte_sig("s", 1, 8)];
    d.procs = vec![
        ProcSpec {
            name: "driver",
            func: driver,
            drives: vec![(0, 1)],
        },
        ProcSpec {
            name: "w",
            func: w,
            drives: vec![],
        },
    ];
    let (lines, r) = d.run(Time::ns(1000));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    let reports: Vec<&String> = lines.iter().filter(|l| l.contains("Note")).collect();
    assert_eq!(
        reports,
        [
            "10 ns +0 p1 Note: t1",
            "15 ns +0 p1 Note: in procedure",
            "30 ns +0 p1 Note: woke at 30000000 fs",
            "50 ns +0 p1 Note: timed out at 50000000 fs",
        ],
        "{lines:#?}"
    );
}

/// Overflow inside a function: a runtime error whose trace is the VHDL call stack.
#[test]
fn checked_overflow_stack_trace() {
    let mut pb = ProgramBuilder::new();
    let sq = pb.declare("square", FuncKind::Function, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let (s_sq, s_call);
    {
        let mut fb = pb.define(sq);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        s_sq = stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        let r = bin(&mut fb, IntOp::Mul, v, v);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(p);
        s_call = stmt(&mut fb, StmtKind::VarAssign);
        let big = fb.int(1 << 40);
        let args = fb.regs(&[big]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: sq,
            args,
            dst: Some(r),
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "top.p",
        func: p,
        drives: vec![],
    }];
    let (_, r) = d.run(Time::ns(10));
    let e = r.unwrap_err();
    let ErrorKind::Runtime(m) = &e.kind else {
        panic!("{e}")
    };
    assert!(m.contains("cannot be represented in 64 bits"), "{m}");
    assert_eq!(e.trace, [s_sq, s_call]);
    assert_eq!(e.process.as_ref().map(|p| p.1.as_str()), Some("top.p"));
    assert_eq!(e.exit_status(), 1);
}

fn lv(n: usize, f: impl Fn(usize) -> u8) -> LogicVec {
    LogicVec::from_fn(n, |i| yavs_logic::StdULogic::from_code_pub(f(i)))
}

fn runs(v: &LogicVec) -> ConstData {
    let mut out: Vec<(u64, u8)> = Vec::new();
    for e in v.iter() {
        let c = e as u8;
        match out.last_mut() {
            Some((n, last)) if *last == c => *n += 1,
            _ => out.push((1, c)),
        }
    }
    ConstData::Logic(out)
}

/// Bit-plane logic arrays across a block boundary: and/xor/not, numeric add and compare,
/// unaligned slice copy, element get/set, driven onto logic signals.
#[test]
fn bit_plane_vector_ops() {
    const N: usize = 70;
    const BYTES: i64 = 64;
    let a = lv(N, |i| if i < 35 { 3 } else { 2 });
    let b = lv(N, |i| match i {
        0..20 => 2,
        20..50 => 3,
        _ => 0,
    });
    let x = LogicVec::from_u64(N, 12345);
    let y = LogicVec::from_u64(N, 777);
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let consts: Vec<ConstId> = [&a, &b, &x, &y].iter().map(|v| pb.konst(runs(v))).collect();
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let slots: Vec<u32> = (0..6).map(|_| fb.slot(BYTES as u32, 8)).collect();
        stmt(&mut fb, StmtKind::VarAssign);
        let addr: Vec<Reg> = slots
            .iter()
            .map(|&s| fb.addr(AddrSpace::Frame, s))
            .collect();
        for (k, &c) in consts.iter().enumerate() {
            fb.emit(Inst::CopyConst { dst: addr[k], c });
        }
        let n = fb.int(N as i64);
        // r = (a and b) xor a, in slot 4.
        fb.emit(Inst::LogicOp {
            op: LOp::And,
            dst: addr[4],
            a: addr[0],
            b: addr[1],
            n,
        });
        fb.emit(Inst::LogicOp {
            op: LOp::Xor,
            dst: addr[4],
            a: addr[4],
            b: addr[0],
            n,
        });
        // r(60 .. 69) := a(30 .. 39); r(5) := 'Z'.
        let (d0, s0, ten) = (fb.int(60), fb.int(30), fb.int(10));
        fb.emit(Inst::LCopy {
            dst: addr[4],
            dst_off: d0,
            src: addr[0],
            src_off: s0,
            n: ten,
        });
        let (i5, z) = (fb.int(5), fb.int(4));
        fb.emit(Inst::LSet {
            base: addr[4],
            idx: i5,
            v: z,
        });
        // slot 5 = x + y; not b into slot 1.
        fb.emit(Inst::LNum {
            op: LNumOp::Add,
            dst: addr[5],
            a: addr[2],
            b: addr[3],
            n,
            signed: false,
        });
        fb.emit(Inst::LogicOp {
            op: LOp::Not,
            dst: addr[1],
            a: addr[1],
            b: addr[1],
            n,
        });
        stmt(&mut fb, StmtKind::Report);
        let a5 = fb.addr(AddrSpace::Frame, slots[4]);
        let i5 = fb.int(5);
        let g = fb.reg(RegTy::I64);
        fb.emit(Inst::LGet {
            dst: g,
            base: a5,
            idx: i5,
        });
        report_image(&mut fb, "r(5) code ", int, g);
        let (xa, ya) = (
            fb.addr(AddrSpace::Frame, slots[2]),
            fb.addr(AddrSpace::Frame, slots[3]),
        );
        let n = fb.int(N as i64);
        let lt = fb.reg(RegTy::I64);
        fb.emit(Inst::LNum {
            op: LNumOp::Cmp(CmpOp::Gt),
            dst: lt,
            a: xa,
            b: ya,
            n,
            signed: false,
        });
        report_image(&mut fb, "x > y = ", int, lt);
        stmt(&mut fb, StmtKind::SigAssign);
        let r = fb.addr(AddrSpace::Frame, slots[4]);
        drive_mem(&mut fb, 0, 0, r, BYTES);
        stmt(&mut fb, StmtKind::SigAssign);
        let s = fb.addr(AddrSpace::Frame, slots[5]);
        drive_mem(&mut fb, 1, 0, s, BYTES);
        stmt(&mut fb, StmtKind::SigAssign);
        let nb = fb.addr(AddrSpace::Frame, slots[1]);
        drive_mem(&mut fb, 2, 0, nb, BYTES);
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    let u = |name| {
        (
            name,
            SignalType::logic(N as u32),
            Value::Logic(LogicVec::new(N, yavs_logic::StdULogic::from_code_pub(0))),
        )
    };
    d.signals = vec![u("r"), u("sum"), u("nb")];
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![(0, N as u32), (1, N as u32), (2, N as u32)],
    }];
    let (lines, r) = d.run(Time::ns(10));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    assert!(has(&lines, "r(5) code 4"), "{lines:#?}");
    assert!(has(&lines, "x > y = 1"), "{lines:#?}");
    let mut want = a.and(&b).xor(&a);
    for k in 0..10 {
        want.set(60 + k, a.get(30 + k));
    }
    want.set(5, yavs_logic::StdULogic::from_code_pub(4));
    let (sim, _, _) = d.build(1);
    drop(sim);
    let (mut sim, _, _) = d.build(4);
    sim.run_until(Time::ns(10)).unwrap();
    let get = |sim: &SimControl, i| sim.value(SignalId(i)).unwrap();
    assert_eq!(get(&sim, 0), Value::Logic(want));
    assert_eq!(
        get(&sim, 1),
        Value::Logic(LogicVec::from_u64(N, 12345 + 777))
    );
    assert_eq!(get(&sim, 2), Value::Logic(b.not()));
}

/// `new`, `.all` read and write, `deallocate`, and the stale-handle error through a copy of
/// the freed access value.
#[test]
fn access_types_and_stale_handle() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("q.all", Dir::To);
    let p = pb.declare("p", FuncKind::Process, None);
    let s_stale;
    {
        let mut fb = pb.define(p);
        let ps = fb.local("p", acc, 8, 8);
        let qs = fb.local("q", acc, 8, 8);
        fb.root(ps, RootKind::Handle);
        fb.root(qs, RootKind::Handle);
        stmt(&mut fb, StmtKind::VarAssign);
        let eight = fb.int(8);
        let init = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc {
            dst: init,
            size: eight,
        });
        let v41 = fb.int(41);
        fb.emit(Inst::Store {
            addr: init,
            src: v41,
            w: Width::B8,
        });
        let h = fb.reg(RegTy::I64);
        fb.emit(Inst::New {
            dst: h,
            ty: int,
            init,
            size: eight,
            shared: false,
        });
        store(&mut fb, AddrSpace::Frame, ps, h, Width::B8);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        let obj = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: obj,
            handle: h,
            what,
        });
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: obj,
            w: Width::B8,
        });
        let one = fb.int(1);
        let v = bin(&mut fb, IntOp::Add, v, one);
        fb.emit(Inst::Store {
            addr: obj,
            src: v,
            w: Width::B8,
        });
        stmt(&mut fb, StmtKind::Report);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        let obj = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: obj,
            handle: h,
            what,
        });
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: obj,
            w: Width::B8,
        });
        report_image(&mut fb, "p.all = ", int, v);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        store(&mut fb, AddrSpace::Frame, qs, h, Width::B8);
        stmt(&mut fb, StmtKind::Call);
        let pa = fb.addr(AddrSpace::Frame, ps);
        fb.emit(Inst::Free { ptr: pa });
        stmt(&mut fb, StmtKind::Report);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        report_image(&mut fb, "p after deallocate = ", int, h);
        s_stale = stmt(&mut fb, StmtKind::Report);
        let h = load(&mut fb, AddrSpace::Frame, qs, Width::B8);
        let obj = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: obj,
            handle: h,
            what,
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (lines, r) = d.run(Time::ns(10));
    assert!(has(&lines, "p.all = 42"), "{lines:#?}");
    assert!(has(&lines, "p after deallocate = 0"), "{lines:#?}");
    let e = r.unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime("access value designates an object that was deallocated: q.all".into())
    );
    assert_eq!(e.trace, [s_stale]);
}

fn new_int(fb: &mut FuncBuilder<'_>, int: TyId, v: i64) -> Reg {
    let eight = fb.int(8);
    let init = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc {
        dst: init,
        size: eight,
    });
    let vr = fb.int(v);
    fb.emit(Inst::Store {
        addr: init,
        src: vr,
        w: Width::B8,
    });
    let h = fb.reg(RegTy::I64);
    fb.emit(Inst::New {
        dst: h,
        ty: int,
        init,
        size: eight,
        shared: false,
    });
    h
}

fn deref(fb: &mut FuncBuilder<'_>, h: Reg, what: CheckMsgId) -> Reg {
    let obj = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Deref {
        dst: obj,
        handle: h,
        what,
    });
    obj
}

/// An address taken before `deallocate` meets the freed object even when `new` runs in the
/// same statement: the slot is not reused before the process suspends (#81).
#[test]
fn freed_slot_not_reused_within_a_statement() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("p.all", Dir::To);
    let p = pb.declare("p", FuncKind::Process, None);
    let s;
    {
        let mut fb = pb.define(p);
        let ps = fb.local("p", acc, 8, 8);
        let qs = fb.local("q", acc, 8, 8);
        fb.root(ps, RootKind::Handle);
        fb.root(qs, RootKind::Handle);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new_int(&mut fb, int, 41);
        store(&mut fb, AddrSpace::Frame, ps, h, Width::B8);
        s = stmt(&mut fb, StmtKind::VarAssign);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        let obj = deref(&mut fb, h, what);
        let pa = fb.addr(AddrSpace::Frame, ps);
        fb.emit(Inst::Free { ptr: pa });
        let h = new_int(&mut fb, int, 2);
        store(&mut fb, AddrSpace::Frame, qs, h, Width::B8);
        let v = fb.int(5);
        fb.emit(Inst::Store {
            addr: obj,
            src: v,
            w: Width::B8,
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (_, r) = d.run(Time::ns(10));
    let e = r.unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime("access to a deallocated object".into())
    );
    assert_eq!(e.trace, [s]);
}

/// An object reached only through an address in a frame slot survives a collection at a
/// wait (a by-reference parameter of a procedure that waits keeps such an address).
#[test]
fn gc_keeps_objects_reached_by_address() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("p.all", Dir::To);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let ps = fb.local("p", acc, 8, 8);
        let addr = fb.local("a", int, 8, 8);
        fb.root(ps, RootKind::Handle);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new_int(&mut fb, int, 41);
        let obj = deref(&mut fb, h, what);
        store(&mut fb, AddrSpace::Frame, addr, obj, Width::B8);
        // Garbage to push the heap over the GC threshold.
        for _ in 0..4 {
            let h = new_int(&mut fb, int, 0);
            store(&mut fb, AddrSpace::Frame, ps, h, Width::B8);
        }
        wait_for(&mut fb, NS);
        stmt(&mut fb, StmtKind::Report);
        let slot = fb.addr(AddrSpace::Frame, addr);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: a,
            addr: slot,
            w: Width::B8,
        });
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: a,
            w: Width::B8,
        });
        report_image(&mut fb, "a.all = ", int, v);
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let b = Bindings {
        limits: Limits {
            gc_threshold: 64,
            ..Limits::default()
        },
        ..Bindings::default()
    };
    let rt = Runtime::new(d.prog.clone(), b).unwrap();
    let (mut sim, t, _) = d.build_rt(1, rt, |p, _| p);
    assert_eq!(
        sim.run_until(Time::ns(10)),
        Ok(RunStatus::Ended(EndReason::Idle))
    );
    let lines = t.0.lock().unwrap().clone();
    assert!(has(&lines, "a.all = 41"), "{lines:#?}");
}

/// Garbage collected at suspensions keeps everything reachable from the root map.
#[test]
fn gc_keeps_roots() {
    gc_keeps_roots_run(true);
}

/// The same loop without a wait: the heap is collected inside the activation.
#[test]
fn gc_inside_an_activation_keeps_roots() {
    gc_keeps_roots_run(false);
}

fn gc_keeps_roots_run(waits: bool) {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("keep.all", Dir::To);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let keep = fb.local("keep", acc, 8, 8);
        let junk = fb.local("junk", acc, 8, 8);
        let i = fb.local("i", int, 8, 8);
        fb.root(keep, RootKind::Handle);
        let new = |fb: &mut FuncBuilder<'_>, v: i64| {
            let eight = fb.int(8);
            let init = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Alloc {
                dst: init,
                size: eight,
            });
            let vr = fb.int(v);
            fb.emit(Inst::Store {
                addr: init,
                src: vr,
                w: Width::B8,
            });
            let h = fb.reg(RegTy::I64);
            fb.emit(Inst::New {
                dst: h,
                ty: int,
                init,
                size: eight,
                shared: false,
            });
            h
        };
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new(&mut fb, 7);
        store(&mut fb, AddrSpace::Frame, keep, h, Width::B8);
        let (lp, body, done) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::Br { to: lp });
        fb.at(lp);
        stmt(&mut fb, StmtKind::Loop);
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let lim = fb.int(200);
        let c = cmp(&mut fb, CmpOp::Lt, iv, lim);
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new(&mut fb, 99);
        store(&mut fb, AddrSpace::Frame, junk, h, Width::B8);
        if waits {
            wait_for(&mut fb, NS);
        }
        stmt(&mut fb, StmtKind::VarAssign);
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let one = fb.int(1);
        let iv = bin(&mut fb, IntOp::Add, iv, one);
        store(&mut fb, AddrSpace::Frame, i, iv, Width::B8);
        fb.emit(Inst::Br { to: lp });
        fb.at(done);
        stmt(&mut fb, StmtKind::Report);
        let h = load(&mut fb, AddrSpace::Frame, keep, Width::B8);
        let obj = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: obj,
            handle: h,
            what,
        });
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: obj,
            w: Width::B8,
        });
        report_image(&mut fb, "keep.all = ", int, v);
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let b = Bindings {
        limits: Limits {
            gc_threshold: 64,
            ..Limits::default()
        },
        ..Bindings::default()
    };
    let rt = Runtime::new(d.prog.clone(), b).unwrap();
    let (mut sim, t, _) = d.build_rt(1, rt, |p, _| p);
    assert_eq!(
        sim.run_until(Time::ns(1000)),
        Ok(RunStatus::Ended(EndReason::Idle))
    );
    let lines = t.0.lock().unwrap().clone();
    assert!(
        lines.iter().any(|l| l.contains("Note: keep.all = 7")),
        "{lines:#?}"
    );
}

/// The shared heap under churn (#303): a serial process replaces the object of a shared
/// variable 300 times without deallocate. Collected at the safepoints between cycles, the
/// heap stays bounded while the objects a shared variable (`keep`) and the process's own
/// variable (`mine`) designate survive; deallocating `keep` afterwards still works, and the
/// output is the same at 1 and 4 threads.
#[test]
fn shared_heap_collected_at_safepoints() {
    use std::sync::{Arc, Mutex};
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("keep.all", Dir::To);
    // The shared variables area: `keep` at 0, `junk` at 8.
    pb.shared_root(0, RootKind::Handle);
    pb.shared_root(8, RootKind::Handle);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let mine = fb.local("mine", acc, 8, 8);
        let i = fb.local("i", int, 8, 8);
        fb.root(mine, RootKind::Handle);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new_int(&mut fb, int, 7);
        store(&mut fb, AddrSpace::Shared, 0, h, Width::B8);
        let h = new_int(&mut fb, int, 5);
        store(&mut fb, AddrSpace::Frame, mine, h, Width::B8);
        let (lp, body, done) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::Br { to: lp });
        fb.at(lp);
        stmt(&mut fb, StmtKind::Loop);
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let lim = fb.int(300);
        let c = cmp(&mut fb, CmpOp::Lt, iv, lim);
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new_int(&mut fb, int, 99);
        store(&mut fb, AddrSpace::Shared, 8, h, Width::B8);
        wait_for(&mut fb, NS);
        stmt(&mut fb, StmtKind::VarAssign);
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let one = fb.int(1);
        let iv = bin(&mut fb, IntOp::Add, iv, one);
        store(&mut fb, AddrSpace::Frame, i, iv, Width::B8);
        fb.emit(Inst::Br { to: lp });
        fb.at(done);
        for (space, off, text) in [
            (AddrSpace::Shared, 0, "keep.all = "),
            (AddrSpace::Frame, mine, "mine.all = "),
        ] {
            stmt(&mut fb, StmtKind::Report);
            let h = load(&mut fb, space, off, Width::B8);
            let obj = deref(&mut fb, h, what);
            let v = fb.reg(RegTy::I64);
            fb.emit(Inst::Load {
                dst: v,
                addr: obj,
                w: Width::B8,
            });
            report_image(&mut fb, text, int, v);
        }
        stmt(&mut fb, StmtKind::Call);
        let a = fb.addr(AddrSpace::Shared, 0);
        fb.emit(Inst::Free { ptr: a });
        stmt(&mut fb, StmtKind::Report);
        let v = load(&mut fb, AddrSpace::Shared, 0, Width::B8);
        report_image(&mut fb, "keep after deallocate = ", int, v);
        wait_forever(&mut fb);
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    assert!(yavs_interp::proc_flags(&prog, p).serial);
    let b = Bindings {
        limits: Limits {
            gc_threshold: 64,
            ..Limits::default()
        },
        ..Bindings::default()
    };
    let rt = Runtime::new(prog.clone(), b).unwrap();
    let mut out = Vec::new();
    for threads in [1, 4] {
        let shared = Arc::new(Mutex::new(yavs_interp::SharedState::new(vec![0; 16])));
        let mut b = Builder::new();
        let ip = yavs_interp::InterpProcess::new(
            rt.clone(),
            p,
            Arc::new(yavs_interp::InstData::default()),
            vec![],
            Some(shared.clone()),
        )
        .unwrap();
        b.process("p", yavs_interp::proc_flags(&prog, p), &[], ip);
        let mut sim = b
            .build(Config {
                threads,
                ..Config::default()
            })
            .unwrap();
        sim.subscribe(Box::new(yavs_interp::SharedGc {
            rt: rt.clone(),
            shared: shared.clone(),
        }));
        let t = Trace::default();
        sim.subscribe(Box::new(t.clone()));
        let mut most = 0;
        for k in 1..=310 {
            sim.run_until(Time::ns(k)).unwrap();
            most = most.max(shared.lock().unwrap().heap_live());
        }
        // 302 objects without collection; a few between two collections.
        assert!(most <= 8, "{most} objects alive");
        out.push(t.0.lock().unwrap().clone());
    }
    assert_eq!(out[0], out[1]);
    assert_eq!(
        out[0],
        [
            "300 ns +0 p0 Note: keep.all = 7",
            "300 ns +0 p0 Note: mine.all = 5",
            "300 ns +0 p0 Note: keep after deallocate = 0"
        ]
    );
}

/// A serial process that allocates and deallocates in a loop without a wait (#355): freed
/// slots are recycled inside the activation, so the shared heap stays at a few thousand slots
/// instead of one per iteration, and an address held in a register across the loop to a
/// slot freed before it still meets the freed object (the stale-address error). Same
/// at 1 and 4 threads.
#[test]
fn freed_slots_recycled_within_an_activation() {
    use std::sync::{Arc, Mutex};
    const N: i64 = 20_000;
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let acc = pb.ty(Ty::Access { target: int });
    let what = pb.check_msg("p.all", Dir::To);
    let p = pb.declare("p", FuncKind::Process, None);
    let s;
    {
        let mut fb = pb.define(p);
        let ps = fb.local("p", acc, 8, 8);
        let qs = fb.local("q", acc, 8, 8);
        let i = fb.local("i", int, 8, 8);
        fb.root(ps, RootKind::Handle);
        fb.root(qs, RootKind::Handle);
        stmt(&mut fb, StmtKind::VarAssign);
        // A shared variable access makes the process serial.
        let zero = fb.int(0);
        store(&mut fb, AddrSpace::Shared, 0, zero, Width::B8);
        let h = new_int(&mut fb, int, 41);
        store(&mut fb, AddrSpace::Frame, ps, h, Width::B8);
        let h = load(&mut fb, AddrSpace::Frame, ps, Width::B8);
        let obj = deref(&mut fb, h, what);
        let pa = fb.addr(AddrSpace::Frame, ps);
        fb.emit(Inst::Free { ptr: pa });
        let (lp, body, done) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::Br { to: lp });
        fb.at(lp);
        stmt(&mut fb, StmtKind::Loop);
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let lim = fb.int(N);
        let c = cmp(&mut fb, CmpOp::Lt, iv, lim);
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = new_int(&mut fb, int, 99);
        store(&mut fb, AddrSpace::Frame, qs, h, Width::B8);
        let qa = fb.addr(AddrSpace::Frame, qs);
        fb.emit(Inst::Free { ptr: qa });
        let iv = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let one = fb.int(1);
        let iv = bin(&mut fb, IntOp::Add, iv, one);
        store(&mut fb, AddrSpace::Frame, i, iv, Width::B8);
        fb.emit(Inst::Br { to: lp });
        fb.at(done);
        s = stmt(&mut fb, StmtKind::VarAssign);
        let v = fb.int(5);
        fb.emit(Inst::Store {
            addr: obj,
            src: v,
            w: Width::B8,
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    assert!(yavs_interp::proc_flags(&prog, p).serial);
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    for threads in [1, 4] {
        let shared = Arc::new(Mutex::new(yavs_interp::SharedState::new(vec![0; 8])));
        let mut b = Builder::new();
        let ip = yavs_interp::InterpProcess::new(
            rt.clone(),
            p,
            Arc::new(yavs_interp::InstData::default()),
            vec![],
            Some(shared.clone()),
        )
        .unwrap();
        b.process("p", yavs_interp::proc_flags(&prog, p), &[], ip);
        let mut sim = b
            .build(Config {
                threads,
                ..Config::default()
            })
            .unwrap();
        let e = sim.run_until(Time::ns(10)).unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::Runtime("access to a deallocated object".into())
        );
        assert_eq!(e.trace, [s]);
        // One slot per iteration without the recycling; at most twice its trigger with it.
        let slots = shared.lock().unwrap().heap_slots();
        assert!(slots <= 8192, "{slots} slots at {threads} threads");
    }
}

/// `to_string` and `'image` natives on integer, real, enumeration and character values;
/// an unimplemented native names its symbol; an unknown symbol fails early.
#[test]
fn natives() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let real = pb.ty(Ty::Real);
    let boolean = pb.ty(Ty::Enum {
        lits: vec!["false".into(), "true".into()],
        logic: Some(LogicKind::Boolean),
    });
    let ch = pb.ty(Ty::Enum {
        lits: (0..=255u8).map(|c| format!("'{}'", c as char)).collect(),
        logic: None,
    });
    let ts_int = pb.native("yavs.implicit.to_string.integer", Some(int));
    let ts_real = pb.native("yavs.implicit.to_string.real", Some(real));
    let ts_ch = pb.native("yavs.implicit.to_string.character", Some(ch));
    let missing = pb.native("yavs.env.pslassertfailed.void.r_boolean", None);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let call = |fb: &mut FuncBuilder<'_>, n: NativeId, v: Reg, msg: &str| {
            let args = fb.regs(&[v]);
            let fat = fb.reg(RegTy::Ptr);
            fb.emit(Inst::CallNative {
                native: n,
                args,
                dst: Some(fat),
            });
            report_with(fb, msg, fat);
        };
        stmt(&mut fb, StmtKind::Report);
        let v = fb.int(-5);
        call(&mut fb, ts_int, v, "int ");
        stmt(&mut fb, StmtKind::Report);
        let r = fb.reg(RegTy::F64);
        fb.emit(Inst::ConstF { dst: r, v: 0.5 });
        call(&mut fb, ts_real, r, "real ");
        stmt(&mut fb, StmtKind::Report);
        let c = fb.int(b'a' as i64);
        call(&mut fb, ts_ch, c, "char ");
        stmt(&mut fb, StmtKind::Report);
        let t = fb.int(1);
        report_image(&mut fb, "bool ", boolean, t);
        let c = fb.int(b'a' as i64);
        report_image(&mut fb, "char image ", ch, c);
        stmt(&mut fb, StmtKind::Call);
        let args = fb.regs(&[]);
        let fat = fb.reg(RegTy::Ptr);
        fb.emit(Inst::CallNative {
            native: missing,
            args,
            dst: Some(fat),
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (lines, r) = d.run(Time::ns(10));
    for w in [
        "int -5",
        "real 0.5",
        "char a",
        "bool true",
        "char image 'a'",
    ] {
        assert!(has(&lines, &format!("Note: {w}")), "{w}: {lines:#?}");
    }
    let e = r.unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime(
            "native subprogram `yavs.env.pslassertfailed.void.r_boolean` is not implemented in yavs yet"
                .into()
        )
    );
    let mut pb = ProgramBuilder::new();
    pb.native("yavs.env.nonsense.void", None);
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    let e = Runtime::new(prog, Bindings::default()).err().unwrap();
    assert!(e.to_string().contains("yavs.env.nonsense.void"), "{e}");
}

extern "C" fn mix(a: i32, b: f64, c: u8) -> i32 {
    a * 100 + b as i32 * 10 + i32::from(c)
}

/// A VHPIDIRECT call to a Rust `extern "C"` function, an argument that does not fit its C
/// type, and binding errors.
#[test]
fn foreign_call() {
    let foreign = ForeignRef {
        name: "mix".into(),
        symbol: "mix".into(),
        args: vec![
            ForeignArg::Value(CType::I32),
            ForeignArg::Value(CType::F64),
            ForeignArg::Value(CType::U8),
        ],
        ret: Some(CType::I32),
        model: None,
    };
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let x = pb.foreign(foreign.clone());
    let p = pb.declare("p", FuncKind::Process, None);
    let s_bad;
    {
        let mut fb = pb.define(p);
        let call = |fb: &mut FuncBuilder<'_>, a: i64| {
            let a = fb.int(a);
            let b = fb.reg(RegTy::F64);
            fb.emit(Inst::ConstF { dst: b, v: 2.0 });
            let c = fb.int(3);
            let args = fb.regs(&[a, b, c]);
            let r = fb.reg(RegTy::I64);
            fb.emit(Inst::CallForeign {
                foreign: x,
                args,
                dst: Some(r),
            });
            r
        };
        stmt(&mut fb, StmtKind::Report);
        let r = call(&mut fb, 7);
        report_image(&mut fb, "mix = ", int, r);
        stmt(&mut fb, StmtKind::Report);
        let r = call(&mut fb, -7);
        report_image(&mut fb, "mix = ", int, r);
        s_bad = stmt(&mut fb, StmtKind::Report);
        call(&mut fb, 1 << 40);
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let bind = || Bindings {
        foreign: vec![(
            x,
            ForeignFn::from_fn(mix as extern "C" fn(i32, f64, u8) -> i32),
        )],
        ..Bindings::default()
    };
    let mut out = Vec::new();
    for threads in [1, 4] {
        let rt = Runtime::new(d.prog.clone(), bind()).unwrap();
        let (mut sim, t, _) = d.build_rt(threads, rt, |p, _| p);
        let r = sim.run_until(Time::ns(10));
        out.push((t.0.lock().unwrap().clone(), r));
    }
    assert_eq!(out[0], out[1]);
    let (lines, r) = &out[0];
    assert!(has(lines, "mix = 723"), "{lines:#?}");
    assert!(has(lines, "mix = -677"), "{lines:#?}");
    let e = r.clone().unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime(
            "argument 1 of foreign `mix`: 1099511627776 does not fit C type int32_t".into()
        )
    );
    assert_eq!(e.trace, [s_bad]);
    // An unbound foreign subprogram is an error only when called (E0107, tested through the
    // CLI); a mismatched binding fails before simulation.
    Runtime::new(d.prog.clone(), Bindings::default()).unwrap();
    let wrong = Bindings {
        foreign: vec![(x, ForeignFn::from_fn(mix2 as extern "C" fn(i32) -> i32))],
        ..Bindings::default()
    };
    let e = Runtime::new(d.prog.clone(), wrong).err().unwrap();
    assert!(e.to_string().contains("C signature"), "{e}");
}

extern "C" fn mix2(a: i32) -> i32 {
    a
}

/// Elaboration-time evaluation without a kernel; signal operations are errors there.
#[test]
fn elaboration_call() {
    let mut pb = ProgramBuilder::new();
    let add = pb.declare("add3", FuncKind::Function, None);
    let bad = pb.declare("reads_signal", FuncKind::Thunk, None);
    {
        let mut fb = pb.define(add);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        let three = fb.int(3);
        let r = bin(&mut fb, IntOp::Add, v, three);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(bad);
        stmt(&mut fb, StmtKind::Return);
        read_scalar(&mut fb, 0);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let inst = std::sync::Arc::new(yavs_interp::InstData {
        sigs: vec![yavs_interp::InstSig {
            signal: SignalId(0),
            ty: SignalType::bytes(1, 1),
        }],
        ..Default::default()
    });
    let v = yavs_interp::call(rt.clone(), add, inst.clone(), &[39]).unwrap();
    assert_eq!(v, Some(42));
    let e = yavs_interp::call(rt, bad, inst, &[]).unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime("signal or wait operation during elaboration".into())
    );
    assert_eq!(e.trace.len(), 1);
}

/// A process touching a shared variable is serial and needs the shared state.
#[test]
fn serial_marking() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let bump = pb.declare("bump", FuncKind::Procedure, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let q = pb.declare("q", FuncKind::Process, None);
    {
        let mut fb = pb.define(bump);
        stmt(&mut fb, StmtKind::VarAssign);
        let v = load(&mut fb, AddrSpace::Shared, 0, Width::B8);
        let one = fb.int(1);
        let v = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Shared, 0, v, Width::B8);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    for f in [p, q] {
        let mut fb = pb.define(f);
        stmt(&mut fb, StmtKind::Call);
        let args = fb.regs(&[]);
        fb.emit(Inst::Call {
            func: bump,
            args,
            dst: None,
        });
        stmt(&mut fb, StmtKind::Report);
        let v = load(&mut fb, AddrSpace::Shared, 0, Width::B8);
        report_image(&mut fb, "shared = ", int, v);
        wait_forever(&mut fb);
        fb.finish();
    }
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    assert!(yavs_interp::proc_flags(&prog, p).serial);
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    let inst = std::sync::Arc::new(yavs_interp::InstData::default());
    let e = yavs_interp::InterpProcess::new(rt.clone(), p, inst.clone(), vec![], None)
        .err()
        .unwrap();
    assert!(e.to_string().contains("touches shared state"), "{e}");
    let mut out = Vec::new();
    for threads in [1, 4] {
        let shared =
            std::sync::Arc::new(std::sync::Mutex::new(yavs_interp::SharedState::new(vec![
                0;
                8
            ])));
        let mut b = Builder::new();
        for (name, f) in [("p", p), ("q", q)] {
            let ip = yavs_interp::InterpProcess::new(
                rt.clone(),
                f,
                inst.clone(),
                vec![],
                Some(shared.clone()),
            )
            .unwrap();
            b.process(name, yavs_interp::proc_flags(&prog, f), &[], ip);
        }
        let mut sim = b
            .build(Config {
                threads,
                ..Config::default()
            })
            .unwrap();
        let t = Trace::default();
        sim.subscribe(Box::new(t.clone()));
        sim.run_until(Time::ns(1)).unwrap();
        out.push(t.0.lock().unwrap().clone());
    }
    assert_eq!(out[0], out[1]);
    assert_eq!(
        out[0],
        ["0 fs +0 p0 Note: shared = 1", "0 fs +0 p1 Note: shared = 2"]
    );
}

/// A shared variable of a protected type: the object is created at elaboration in the shared
/// heap, its state record is reached through `self`, and two processes call its methods
/// (serially, in elaboration order).
#[test]
fn protected_counter() {
    use std::sync::{Arc, Mutex};
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let state = pb.ty(Ty::Record {
        fields: vec![Field {
            name: "count".into(),
            ty: int,
            desc: None,
        }],
    });
    let pt = pb.ty(Ty::Protected { state });
    let what = pb.check_msg("counter", Dir::To);
    let make = pb.declare("make_counter", FuncKind::Function, None);
    let incr = pb.declare("counter.incr", FuncKind::Procedure, None);
    let value = pb.declare("counter.value", FuncKind::Function, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let q = pb.declare("q", FuncKind::Process, None);
    {
        let mut fb = pb.define(make);
        stmt(&mut fb, StmtKind::Return);
        let eight = fb.int(8);
        let init = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc {
            dst: init,
            size: eight,
        });
        let h = fb.reg(RegTy::I64);
        fb.emit(Inst::New {
            dst: h,
            ty: pt,
            init,
            size: eight,
            shared: true,
        });
        fb.emit(Inst::Ret { v: Some(h) });
        fb.finish();
    }
    // Both methods: the state record through `self`.
    for (f, step) in [(incr, true), (value, false)] {
        let mut fb = pb.define(f);
        let me = fb.param("self", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = load(&mut fb, AddrSpace::Frame, me, Width::B8);
        let obj = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: obj,
            handle: h,
            what,
        });
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: obj,
            w: Width::B8,
        });
        if step {
            let one = fb.int(1);
            let v = bin(&mut fb, IntOp::Add, v, one);
            fb.emit(Inst::Store {
                addr: obj,
                src: v,
                w: Width::B8,
            });
            fb.emit(Inst::Ret { v: None });
        } else {
            fb.emit(Inst::Ret { v: Some(v) });
        }
        fb.finish();
    }
    for f in [p, q] {
        let mut fb = pb.define(f);
        stmt(&mut fb, StmtKind::Call);
        let obj = load(&mut fb, AddrSpace::Shared, 0, Width::B8);
        let args = fb.regs(&[]);
        fb.emit(Inst::CallProt {
            func: incr,
            obj,
            args,
            dst: None,
        });
        stmt(&mut fb, StmtKind::Report);
        let obj = load(&mut fb, AddrSpace::Shared, 0, Width::B8);
        let args = fb.regs(&[]);
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::CallProt {
            func: value,
            obj,
            args,
            dst: Some(v),
        });
        report_image(&mut fb, "count = ", int, v);
        wait_forever(&mut fb);
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
    let inst = Arc::new(yavs_interp::InstData::default());
    let mut out = Vec::new();
    for threads in [1, 4] {
        let shared = Arc::new(Mutex::new(yavs_interp::SharedState::new(vec![0; 8])));
        // Elaboration: `shared variable counter : counter_pt;`.
        let h = yavs_interp::call_shared(rt.clone(), make, inst.clone(), &[], Some(&shared))
            .unwrap()
            .unwrap();
        shared
            .lock()
            .unwrap()
            .vars
            .copy_from_slice(&h.to_le_bytes());
        let mut b = Builder::new();
        for (name, f) in [("p", p), ("q", q)] {
            assert!(yavs_interp::proc_flags(&prog, f).serial);
            let ip = yavs_interp::InterpProcess::new(
                rt.clone(),
                f,
                inst.clone(),
                vec![],
                Some(shared.clone()),
            )
            .unwrap();
            b.process(name, yavs_interp::proc_flags(&prog, f), &[], ip);
        }
        let mut sim = b
            .build(Config {
                threads,
                ..Config::default()
            })
            .unwrap();
        let t = Trace::default();
        sim.subscribe(Box::new(t.clone()));
        sim.run_until(Time::ns(1)).unwrap();
        out.push(t.0.lock().unwrap().clone());
    }
    assert_eq!(out[0], out[1]);
    assert_eq!(
        out[0],
        ["0 fs +0 p0 Note: count = 1", "0 fs +0 p1 Note: count = 2"]
    );
    // Without the shared state the creator cannot run.
    let e = yavs_interp::call(rt, make, inst, &[]).unwrap_err();
    assert!(matches!(e.kind, ErrorKind::Internal(_)), "{e}");
}

/// `call_values`: a composite argument in, the same array back as a fat-pair result (#77).
#[test]
fn elaboration_call_with_composites() {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int {
        lo: i64::MIN,
        hi: i64::MAX,
    });
    let arr = pb.ty(Ty::Array {
        elem: int,
        elem_desc: None,
        dims: 1,
    });
    let id = pb.declare("id", FuncKind::Function, None);
    {
        let mut fb = pb.define(id);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        fb.param("x'dope", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::Return);
        let src = fb.reg(RegTy::Ptr);
        fb.emit(Inst::AddrOf {
            dst: src,
            space: AddrSpace::Frame,
            off: x,
        });
        let n = fb.int(16);
        let fat = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: fat, size: n });
        fb.emit(Inst::Copy {
            dst: fat,
            src,
            bytes: n,
        });
        fb.emit(Inst::Ret { v: Some(fat) });
        fb.finish();
    }
    let rt = Runtime::new(
        std::sync::Arc::new(pb.finish().unwrap()),
        Bindings::default(),
    )
    .unwrap();
    let data: Vec<u8> = [7i64, 8, 9].iter().flat_map(|x| x.to_le_bytes()).collect();
    let desc: Vec<u8> = [2i64, 0, 1].iter().flat_map(|x| x.to_le_bytes()).collect();
    let v = yavs_interp::CallVal::Composite { data, desc };
    let r = yavs_interp::call_values(
        rt,
        id,
        Default::default(),
        std::slice::from_ref(&v),
        Some(arr),
    );
    assert_eq!(r.unwrap(), Some(v));
}

/// A reference of several parts (`SigParts`, #71): `x(0) => a, x(1 to 2) => b` given out of
/// order, driven, read whole and sliced across the parts, and waited on.
#[test]
fn signal_reference_of_several_parts() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    // `[parts]` with b first: entries [ref, slot, first element].
    let parts = |fb: &mut FuncBuilder<'_>| {
        let size = fb.int(2 * i64::from(SIG_PART_BYTES));
        let t = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: t, size });
        for (k, (s, len, slot, first)) in [(1, 2, 1, 1), (0, 1, 0, 0)].into_iter().enumerate() {
            let r = sig(fb, s, 0, len);
            let at = fb.reg(RegTy::Ptr);
            let o = fb.int(k as i64 * i64::from(SIG_PART_BYTES));
            fb.emit(Inst::Index {
                dst: at,
                base: t,
                pos: o,
                stride: 1,
            });
            fb.emit(Inst::Store {
                addr: at,
                src: r,
                w: Width::Sig,
            });
            for (off, v) in [(16, slot), (24, first)] {
                let c = fb.int(off);
                let a = fb.reg(RegTy::Ptr);
                fb.emit(Inst::Index {
                    dst: a,
                    base: at,
                    pos: c,
                    stride: 1,
                });
                let v = fb.int(v);
                fb.emit(Inst::Store {
                    addr: a,
                    src: v,
                    w: Width::B8,
                });
            }
        }
        let n = fb.int(2);
        let x = fb.reg(RegTy::Sig);
        fb.emit(Inst::SigParts {
            dst: x,
            parts: t,
            n,
        });
        x
    };
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        stmt(&mut fb, StmtKind::SigAssign);
        let x = parts(&mut fb);
        // x <= (10, 20, 30) after 1 ns, through each part's own slot.
        let size = fb.int(32);
        let wave = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: wave, size });
        for (k, v) in [NS, 10, 20, 30].into_iter().enumerate() {
            let at = fb.reg(RegTy::Ptr);
            let o = fb.int(8 * k as i64);
            fb.emit(Inst::Index {
                dst: at,
                base: wave,
                pos: o,
                stride: 1,
            });
            let v = fb.int(v);
            fb.emit(Inst::Store {
                addr: at,
                src: v,
                w: Width::B8,
            });
        }
        let (none, one) = (fb.int(-1), fb.int(1));
        fb.emit(Inst::DriveSlot {
            slot: none,
            target: x,
            wave,
            n: one,
            mode: DriveMode::Transport,
            reject: None,
        });
        wait_forever(&mut fb);
        fb.finish();
    }
    let q = pb.declare("q", FuncKind::Process, None);
    {
        let mut fb = pb.define(q);
        stmt(&mut fb, StmtKind::Report);
        let x = parts(&mut fb);
        let size = fb.int(24);
        let buf = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: buf, size });
        fb.emit(Inst::SigRead { dst: buf, sig: x });
        for k in 0..3 {
            let v = load_at(&mut fb, buf, 8 * k);
            report_image(&mut fb, &format!("x({k}) = "), int, v);
        }
        let (one, two) = (fb.int(1), fb.int(2));
        let tail = fb.reg(RegTy::Sig);
        fb.emit(Inst::SigSlice {
            dst: tail,
            sig: x,
            offset: one,
            len: two,
        });
        let ev = fb.reg(RegTy::I64);
        fb.emit(Inst::SigAttr {
            dst: ev,
            kind: AttrKind::Event,
            sig: tail,
        });
        report_image(&mut fb, "x(1 to 2)'event = ", int, ev);
        stmt(&mut fb, StmtKind::Wait);
        let x = parts(&mut fb);
        fb.emit(Inst::WaitOn { sig: x });
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: BlockId(0) });
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.signals = vec![byte_sig("a", 1, 8), byte_sig("b", 2, 8)];
    d.procs = vec![
        ProcSpec {
            name: "p",
            func: p,
            drives: vec![(0, 1), (1, 2)],
        },
        ProcSpec {
            name: "q",
            func: q,
            drives: vec![],
        },
    ];
    let (lines, r) = d.run(Time::ns(10));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    let reports: Vec<&String> = lines.iter().filter(|l| l.contains("Note")).collect();
    assert_eq!(
        reports,
        [
            "0 fs +0 p1 Note: x(0) = 0",
            "0 fs +0 p1 Note: x(1) = 0",
            "0 fs +0 p1 Note: x(2) = 0",
            "0 fs +0 p1 Note: x(1 to 2)'event = 0",
            "1 ns +0 p1 Note: x(0) = 10",
            "1 ns +0 p1 Note: x(1) = 20",
            "1 ns +0 p1 Note: x(2) = 30",
            "1 ns +0 p1 Note: x(1 to 2)'event = 1",
        ]
    );
}

fn load_at(fb: &mut FuncBuilder<'_>, base: Reg, off: i64) -> Reg {
    let at = fb.reg(RegTy::Ptr);
    let o = fb.int(off);
    fb.emit(Inst::Index {
        dst: at,
        base,
        pos: o,
        stride: 1,
    });
    let v = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: v,
        addr: at,
        w: Width::B8,
    });
    v
}
