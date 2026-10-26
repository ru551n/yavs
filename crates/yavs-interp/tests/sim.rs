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

/// A trial of the compiled tier counts the activations of one process function
/// (`NativeFlags::count_on`, #652), with a tier set that compiles nothing, and stops when asked.
#[test]
fn activations_of_a_process_are_counted_while_a_trial_asks() {
    struct Idle;
    impl yavs_interp::Tier for Idle {
        fn run(&self, _: FuncId, _: u32, _: &mut dyn yavs_interp::TierHost) -> Option<u32> {
            None
        }
    }
    let mut pb = ProgramBuilder::new();
    let proc_ = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(proc_);
        wait_for(&mut fb, 10 * NS);
        fb.emit(Inst::Br { to: BlockId(0) });
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: proc_,
        drives: vec![],
    }];
    let rt = Runtime::new(d.prog.clone(), Default::default()).unwrap();
    assert!(rt.set_tier(std::sync::Arc::new(Idle)));
    let flags = rt.native_flags();
    let (mut sim, _, _) = d.build_rt(1, rt, |p, _| p);
    assert_eq!(flags.entries(), 0);
    flags.count_on(proc_);
    sim.run_until(Time::ns(100)).unwrap();
    let n = flags.entries();
    flags.count_off(proc_);
    sim.run_until(Time::ns(200)).unwrap();
    assert!((9..=11).contains(&n), "{n}");
    assert_eq!(flags.entries(), n);
}

/// A function the tier flags while an interpreted process runs is entered through the tier at
/// the process's next calls (`Pre::native`, seen by the watching loop within an activation or
/// `WATCH` events), and not before it is flagged (#750).
#[test]
fn a_callee_flagged_later_is_entered_through_the_tier() {
    use std::sync::atomic::{AtomicU32, Ordering};
    // Compiled code for `.0` once `.2` is set; counts the entries.
    struct Count(FuncId, AtomicU32, std::sync::atomic::AtomicBool);
    impl yavs_interp::Tier for Count {
        fn run(&self, f: FuncId, _: u32, _: &mut dyn yavs_interp::TierHost) -> Option<u32> {
            if f == self.0 {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            None
        }
        fn ready(&self, f: FuncId) -> bool {
            f == self.0 && self.2.load(Ordering::Relaxed)
        }
    }
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let sq = pb.declare("square", FuncKind::Function, None);
    let proc_ = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(sq);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        // Longer than the optimiser inlines (it would leave no call to flag).
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        let mut r = v;
        for _ in 0..30 {
            r = bin(&mut fb, IntOp::Add, r, v);
        }
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        // A new argument each time: the memo of pure calls would answer a repeated one.
        let mut fb = pb.define(proc_);
        let n = fb.local("n", int, 8, 8);
        wait_for(&mut fb, 10 * NS);
        stmt(&mut fb, StmtKind::Call);
        let v = load(&mut fb, AddrSpace::Frame, n, Width::B8);
        let one = fb.int(1);
        let v1 = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Frame, n, v1, Width::B8);
        let args = fb.regs(&[v1]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: sq,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Br { to: BlockId(0) });
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: proc_,
        drives: vec![],
    }];
    let rt = Runtime::new(d.prog.clone(), Default::default()).unwrap();
    let tier = std::sync::Arc::new(Count(sq, AtomicU32::new(0), Default::default()));
    assert!(rt.set_tier(tier.clone()));
    let flags = rt.native_flags();
    let (mut sim, _, _) = d.build_rt(1, rt, |p, _| p);
    sim.run_until(Time::ns(100)).unwrap();
    assert_eq!(tier.1.load(Ordering::Relaxed), 0);
    tier.2.store(true, Ordering::Relaxed);
    flags.set(sq);
    sim.run_until(Time::ns(200)).unwrap();
    assert!(
        tier.1.load(Ordering::Relaxed) >= 9,
        "{}",
        tier.1.load(Ordering::Relaxed)
    );
    // Compiled code of the callee hands the process over to the loop with the tier.
    assert!(flags.retiers() > 0);
}

/// In the loop with the tier (the process has compiled code) a callee the tier never touched is
/// called with its parameters in registers only and entered past its prologue; one the tier
/// flags while it runs is entered through the tier from there. The trace is the interpreter's
/// (#750).
#[test]
fn a_callee_the_tier_never_touched_is_called_lean_in_the_loop_with_the_tier() {
    struct Late(FuncId, FuncId, std::sync::atomic::AtomicBool);
    impl yavs_interp::Tier for Late {
        fn run(&self, _: FuncId, _: u32, _: &mut dyn yavs_interp::TierHost) -> Option<u32> {
            None
        }
        fn ready(&self, f: FuncId) -> bool {
            f == self.0 || f == self.1 && self.2.load(std::sync::atomic::Ordering::Relaxed)
        }
    }
    let build = || {
        let mut pb = ProgramBuilder::new();
        let int = integer(&mut pb);
        let sq = pb.declare("square", FuncKind::Function, None);
        let proc_ = pb.declare("p", FuncKind::Process, None);
        {
            let mut fb = pb.define(sq);
            let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
            stmt(&mut fb, StmtKind::Return);
            let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
            let mut r = v;
            for _ in 0..30 {
                r = bin(&mut fb, IntOp::Add, r, v);
            }
            fb.emit(Inst::Ret { v: Some(r) });
            fb.finish();
        }
        {
            let mut fb = pb.define(proc_);
            let n = fb.local("n", int, 8, 8);
            wait_for(&mut fb, 10 * NS);
            stmt(&mut fb, StmtKind::Call);
            let v = load(&mut fb, AddrSpace::Frame, n, Width::B8);
            let one = fb.int(1);
            let v1 = bin(&mut fb, IntOp::Add, v, one);
            store(&mut fb, AddrSpace::Frame, n, v1, Width::B8);
            let args = fb.regs(&[v1]);
            let r = fb.reg(RegTy::I64);
            fb.emit(Inst::Call {
                func: sq,
                args,
                dst: Some(r),
            });
            drive(&mut fb, 0, 0, r, Width::B8);
            fb.emit(Inst::Br { to: BlockId(0) });
            fb.finish();
        }
        let mut d = Design::new(pb);
        d.signals = vec![byte_sig("s", 1, 8)];
        d.procs = vec![ProcSpec {
            name: "p",
            func: proc_,
            drives: vec![(0, 1)],
        }];
        (d, sq, proc_)
    };
    let (d, _, _) = build();
    let (want, r) = d.run(Time::ns(200));
    assert!(r.is_ok());
    assert!(has(&want, "s=[3"), "{want:#?}");
    let (d, sq, proc_) = build();
    let rt = Runtime::new(d.prog.clone(), Default::default()).unwrap();
    let tier = std::sync::Arc::new(Late(proc_, sq, Default::default()));
    assert!(rt.set_tier(tier.clone()));
    let flags = rt.native_flags();
    let (mut sim, t, _) = d.build_rt(1, rt, |p, _| p);
    sim.run_until(Time::ns(100)).unwrap();
    tier.2.store(true, std::sync::atomic::Ordering::Relaxed);
    flags.set(sq);
    sim.run_until(Time::ns(200)).unwrap();
    assert_eq!(*t.0.lock().unwrap(), want);
}

type Sampler = Box<dyn Fn(&mut Vec<(FuncId, bool)>) + Send>;

/// A tier whose only function is `.0`, called on a native frame once flagged (`.1` counts the
/// calls; `u32::MAX` while not callable), with no compiled code. `.2` holds a sampler that is
/// asked at every call, and what it reported.
struct Calls(
    FuncId,
    std::sync::atomic::AtomicU32,
    std::sync::Mutex<Option<Sampler>>,
    std::sync::Mutex<Vec<(FuncId, bool)>>,
);

impl yavs_interp::Tier for Calls {
    fn run(&self, _: FuncId, _: u32, _: &mut dyn yavs_interp::TierHost) -> Option<u32> {
        None
    }
    fn ready(&self, _: FuncId) -> bool {
        false
    }
    fn callable(&self, f: FuncId) -> bool {
        f == self.0 && self.1.load(std::sync::atomic::Ordering::Relaxed) != u32::MAX
    }
    fn call(
        &self,
        f: FuncId,
        _: &[u64],
        _: Option<u64>,
        _: u32,
        _: &mut dyn yavs_interp::TierHost,
    ) -> Option<yavs_interp::Called> {
        (f == self.0).then(|| {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Some(sample) = &*self.2.lock().unwrap() {
                sample(&mut self.3.lock().unwrap());
            }
            yavs_interp::Called::Done(0)
        })
    }
}

/// A process that calls `square` once a clock with a new argument each time (the memo of pure
/// calls would answer a repeated one); the second function is `square`.
fn square_caller() -> (Design, FuncId) {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let sq = pb.declare("square", FuncKind::Function, None);
    let proc_ = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(sq);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        let mut r = v;
        for _ in 0..30 {
            r = bin(&mut fb, IntOp::Add, r, v);
        }
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(proc_);
        let n = fb.local("n", int, 8, 8);
        wait_for(&mut fb, 10 * NS);
        stmt(&mut fb, StmtKind::Call);
        let v = load(&mut fb, AddrSpace::Frame, n, Width::B8);
        let one = fb.int(1);
        let v1 = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Frame, n, v1, Width::B8);
        let args = fb.regs(&[v1]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: sq,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Br { to: BlockId(0) });
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: proc_,
        drives: vec![],
    }];
    (d, sq)
}

/// A function the tier only calls on a native frame (`NativeFlags::set_call`, no frame-mode code
/// anywhere, so `NativeFlags::set` never ran) is called through the tier by an interpreted
/// process as soon as it is flagged, whatever order the tier flagged its functions in (#764),
/// by the loop without the tier's bookkeeping: no hand-over (#757).
#[test]
fn a_callee_flagged_for_native_calls_alone_is_called_through_the_tier() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (d, sq) = square_caller();
    let rt = Runtime::new(d.prog.clone(), Default::default()).unwrap();
    // Not callable when the tier is set: the tier flags it while the run goes on.
    let tier = std::sync::Arc::new(Calls(
        sq,
        AtomicU32::new(u32::MAX),
        Default::default(),
        Default::default(),
    ));
    assert!(rt.set_tier(tier.clone()));
    let flags = rt.native_flags();
    let (mut sim, _, _) = d.build_rt(1, rt, |p, _| p);
    sim.run_until(Time::ns(100)).unwrap();
    tier.1.store(0, Ordering::Relaxed);
    flags.set_call(sq);
    sim.run_until(Time::ns(200)).unwrap();
    assert!(
        tier.1.load(Ordering::Relaxed) >= 9,
        "{}",
        tier.1.load(Ordering::Relaxed)
    );
    assert_eq!(flags.retiers(), 0);
}

/// The profile's sampler names a function the tier runs on a native frame for an interpreted
/// caller as itself, compiled, not as its caller (#760).
#[test]
fn the_profile_names_a_native_frame_callee_not_its_caller() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (d, sq) = square_caller();
    let rt = Runtime::new(d.prog.clone(), Default::default()).unwrap();
    let tier = std::sync::Arc::new(Calls(
        sq,
        AtomicU32::new(0),
        Default::default(),
        Default::default(),
    ));
    *tier.2.lock().unwrap() = Some(Box::new(rt.profile_sampler()));
    assert!(rt.set_tier(tier.clone()));
    let (mut sim, _, _) = d.build_rt(1, rt, |p, _| p);
    sim.run_until(Time::ns(100)).unwrap();
    let seen = tier.3.lock().unwrap().clone();
    assert!(tier.1.load(Ordering::Relaxed) >= 9);
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|&s| s == (sq, true)), "{seen:?}");
}

/// Counters of rising edges, one small (its registers stay with the process between
/// activations) and one whose temporaries outgrow what a process keeps (it hands its vectors
/// to the thread's scratch), both run for many activations with identical results.
#[test]
fn processes_with_resident_and_scratch_vectors() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let clkgen = pb.declare("clkgen", FuncKind::Process, None);
    let small = pb.declare("small", FuncKind::Process, None);
    let small2 = pb.declare("small2", FuncKind::Process, None);
    let big = pb.declare("big", FuncKind::Process, None);
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
    for (f, temp) in [(small, 0), (big, 4096), (small2, 0)] {
        let mut fb = pb.define(f);
        let count = fb.local("count", int, 8, 8);
        let (b1, b2) = (fb.block(), fb.block());
        stmt(&mut fb, StmtKind::If);
        let c = read_scalar(&mut fb, 0);
        let one = fb.int(1);
        let e = cmp(&mut fb, CmpOp::Eq, c, one);
        fb.emit(Inst::CondBr { c: e, t: b1, f: b2 });
        fb.at(b1);
        stmt(&mut fb, StmtKind::VarAssign);
        if temp > 0 {
            // A temporary bigger than a process keeps, written and read back.
            let size = fb.int(temp);
            let t = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Alloc { dst: t, size });
            let last = fb.int(temp - 8);
            let at = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Index {
                dst: at,
                base: t,
                pos: last,
                stride: 1,
            });
            let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
            fb.emit(Inst::Store {
                addr: at,
                src: v,
                w: Width::B8,
            });
        }
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        let one = fb.int(1);
        let v1 = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Frame, count, v1, Width::B8);
        stmt(&mut fb, StmtKind::SigAssign);
        let v = load(&mut fb, AddrSpace::Frame, count, Width::B8);
        drive(&mut fb, 0, 0, v, Width::B8);
        fb.emit(Inst::Br { to: b2 });
        fb.at(b2);
        wait_on_loop(&mut fb, 0, BlockId(0));
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.signals = vec![
        byte_sig("clk", 1, 1),
        byte_sig("small", 1, 8),
        byte_sig("big", 1, 8),
        byte_sig("small2", 1, 8),
    ];
    d.procs = vec![
        ProcSpec {
            name: "clkgen",
            func: clkgen,
            drives: vec![(0, 1)],
        },
        ProcSpec {
            name: "small",
            func: small,
            drives: vec![(1, 1)],
        },
        ProcSpec {
            name: "small2",
            func: small2,
            drives: vec![(3, 1)],
        },
        ProcSpec {
            name: "big",
            func: big,
            drives: vec![(2, 1)],
        },
    ];
    let (lines, r) = d.run(Time::ns(1000));
    assert!(r.is_ok(), "{r:?}");
    // 1000 ns of a 10 ns clock: 100 rising edges (the last at 995 ns).
    assert!(
        has(&lines, "995 ns +1 big=[100, 0, 0, 0, 0, 0, 0, 0]"),
        "{lines:#?}"
    );
    for name in ["small", "small2"] {
        let want = format!("995 ns +1 {name}=[100, 0, 0, 0, 0, 0, 0, 0]");
        assert!(has(&lines, &want), "{lines:#?}");
    }
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

/// A 4-byte load of a negative value sign-extends, through the fused `LoadSlot`
/// and `LoadInst` ops as through the generic `Load` (#746).
#[test]
fn narrow_loads_sign_extend() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let frame = pb.declare("frame", FuncKind::Function, None);
    let inst = pb.declare("inst", FuncKind::Function, None);
    {
        // `frame(x)`: store the parameter as B4 in a local, load it back (LoadSlot).
        let mut fb = pb.define(frame);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        let slot = fb.local("slot", int, 8, 8);
        stmt(&mut fb, StmtKind::VarAssign);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        store(&mut fb, AddrSpace::Frame, slot, v, Width::B4);
        let got = load(&mut fb, AddrSpace::Frame, slot, Width::B4);
        fb.emit(Inst::Ret { v: Some(got) });
        fb.finish();
    }
    {
        // `inst()`: a constant B4 load of instance data (LoadInst).
        let mut fb = pb.define(inst);
        stmt(&mut fb, StmtKind::Return);
        let got = load(&mut fb, AddrSpace::Inst, 0, Width::B4);
        fb.emit(Inst::Ret { v: Some(got) });
        fb.finish();
    }
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    for v in [-1i32, i32::MIN] {
        let want = Some(i64::from(v) as u64);
        let got = yavs_interp::call(
            rt.clone(),
            frame,
            std::sync::Arc::new(yavs_interp::InstData::default()),
            &[i64::from(v) as u64],
        )
        .unwrap();
        assert_eq!(got, want, "frame {v}");
        let data = std::sync::Arc::new(yavs_interp::InstData {
            data: v.to_le_bytes().to_vec(),
            ..Default::default()
        });
        let got = yavs_interp::call(rt.clone(), inst, data, &[]).unwrap();
        assert_eq!(got, want, "inst {v}");
    }
}

/// A function `lin(x, y) = y - (x * 16 + 3) - 1`: the fast loop runs the four `Bin`s as one
/// linear op for operands within 2^30 and as the checked instructions beyond, with the same
/// results and the same overflow error at the failing statement.
fn lin_design(args: &[(i64, i64)]) -> (Vec<String>, Result<RunStatus, SimError>, [StmtId; 2]) {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let lin = pb.declare("lin", FuncKind::Function, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let s_lin;
    {
        let mut fb = pb.define(lin);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
        let y = fb.param("y", Width::B8, ParamClass::Value, Mode::In, None);
        s_lin = stmt(&mut fb, StmtKind::Return);
        let (xv, yv) = (
            load(&mut fb, AddrSpace::Frame, x, Width::B8),
            load(&mut fb, AddrSpace::Frame, y, Width::B8),
        );
        let (c16, c3, c1) = (fb.int(16), fb.int(3), fb.int(1));
        let t1 = bin(&mut fb, IntOp::Mul, xv, c16);
        let t2 = bin(&mut fb, IntOp::Add, t1, c3);
        let t3 = bin(&mut fb, IntOp::Sub, yv, t2);
        let t4 = bin(&mut fb, IntOp::Sub, t3, c1);
        fb.emit(Inst::Ret { v: Some(t4) });
        fb.finish();
    }
    let s_call;
    {
        let mut fb = pb.define(p);
        s_call = stmt(&mut fb, StmtKind::Report);
        for &(x, y) in args {
            let (xr, yr) = (fb.int(x), fb.int(y));
            let a = fb.regs(&[xr, yr]);
            let r = fb.reg(RegTy::I64);
            fb.emit(Inst::Call {
                func: lin,
                args: a,
                dst: Some(r),
            });
            report_image(&mut fb, "lin = ", int, r);
        }
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "top.p",
        func: p,
        drives: vec![],
    }];
    let (lines, r) = d.run(Time::ns(10));
    (lines, r, [s_lin, s_call])
}

#[test]
fn linear_runs_match_checked_arithmetic() {
    let (lines, r, _) = lin_design(&[(5, 100), (1 << 40, 7), (-(1 << 35), -9)]);
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    let got: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.split("lin = ").nth(1))
        .collect();
    let want = |x: i64, y: i64| (y - (x * 16 + 3) - 1).to_string();
    assert_eq!(
        got,
        [want(5, 100), want(1 << 40, 7), want(-(1 << 35), -9)],
        "{lines:#?}"
    );
}

#[test]
fn linear_run_overflow_is_the_checked_error() {
    let (_, r, [s_lin, s_call]) = lin_design(&[(1 << 62, 1)]);
    let e = r.unwrap_err();
    let ErrorKind::Runtime(m) = &e.kind else {
        panic!("{e}")
    };
    assert!(m.contains("cannot be represented in 64 bits"), "{m}");
    assert_eq!(e.trace, [s_lin, s_call]);
}

/// `off(i) = len * 16 - (pos(i) * 16 + 3) - 1` over the descriptor `7 downto 0`: the fast loop
/// fuses `Pos`, `DopeLen` and the four `Bin`s into one op; an index outside the range is the
/// error of the `Pos` itself.
fn elem_design(idxs: &[i64]) -> (Vec<String>, Result<RunStatus, SimError>) {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let what = pb.check_msg("array V", Dir::Downto);
    let off = pb.declare("off", FuncKind::Function, None);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(off);
        let slot = fb.param("i", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::Return);
        let data = fb.addr(AddrSpace::Frame, 0);
        let fat = fat_of(&mut fb, data, 7, 0, true);
        let (_, _) = fat_parts(&mut fb, fat);
        let eight = fb.int(8);
        let pd = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: pd,
            base: fat,
            pos: eight,
            stride: 1,
        });
        let dope = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: dope,
            addr: pd,
            w: Width::B8,
        });
        let i = load(&mut fb, AddrSpace::Frame, slot, Width::B8);
        let pos = fb.reg(RegTy::I64);
        fb.emit(Inst::Pos {
            dst: pos,
            dope,
            dim: 0,
            idx: i,
            what,
        });
        let len = fb.reg(RegTy::I64);
        fb.emit(Inst::DopeLen {
            dst: len,
            dope,
            dim: 0,
        });
        let (c16, c3, c1) = (fb.int(16), fb.int(3), fb.int(1));
        let t1 = bin(&mut fb, IntOp::Mul, pos, c16);
        let t2 = bin(&mut fb, IntOp::Add, t1, c3);
        let t3 = bin(&mut fb, IntOp::Mul, len, c16);
        let t4 = bin(&mut fb, IntOp::Sub, t3, t2);
        let t5 = bin(&mut fb, IntOp::Sub, t4, c1);
        fb.emit(Inst::Ret { v: Some(t5) });
        fb.finish();
    }
    {
        let mut fb = pb.define(p);
        stmt(&mut fb, StmtKind::Report);
        for &i in idxs {
            let ir = fb.int(i);
            let a = fb.regs(&[ir]);
            let r = fb.reg(RegTy::I64);
            fb.emit(Inst::Call {
                func: off,
                args: a,
                dst: Some(r),
            });
            report_image(&mut fb, "off = ", int, r);
        }
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "top.p",
        func: p,
        drives: vec![],
    }];
    d.run(Time::ns(10))
}

#[test]
fn position_and_length_runs_match_the_instructions() {
    let (lines, r) = elem_design(&[7, 3, 0]);
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    let got: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.split("off = ").nth(1))
        .collect();
    let want = |i: i64| (8 * 16 - ((7 - i) * 16 + 3) - 1).to_string();
    assert_eq!(got, [want(7), want(3), want(0)], "{lines:#?}");
}

#[test]
fn position_and_length_run_with_an_index_outside_is_the_pos_error() {
    let (lines, r) = elem_design(&[2, 9]);
    let e = r.unwrap_err();
    let ErrorKind::Runtime(m) = &e.kind else {
        panic!("{e}")
    };
    assert!(m.contains("index 9 outside of range 7 downto 0"), "{m}");
    assert!(has(&lines, "off = "), "{lines:#?}");
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
        let h = fb.reg(RegTy::Handle);
        fb.emit(Inst::New {
            dst: h,
            ty: int,
            init,
            size: eight,
            shared: false,
        });
        store(&mut fb, AddrSpace::Frame, ps, h, Width::B8);
        stmt(&mut fb, StmtKind::VarAssign);
        let h = load_handle(&mut fb, AddrSpace::Frame, ps);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, ps);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, qs);
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
    let h = fb.reg(RegTy::Handle);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, ps);
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

/// An object reached only through an address in a frame slot (`RootKind::Addr`) survives a
/// collection at a wait (a by-reference parameter of a procedure that waits keeps such an
/// address).
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
        fb.root(addr, RootKind::Addr);
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
            let h = fb.reg(RegTy::Handle);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, keep);
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
            let h = load_handle(&mut fb, space, off);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, ps);
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
        // The `gc-stress` build never reuses a slot, so there only the live objects count.
        let s = shared.lock().unwrap();
        if cfg!(feature = "gc-stress") {
            assert!(
                s.heap_live() <= 8,
                "{} live at {threads} threads",
                s.heap_live()
            );
        } else {
            assert!(
                s.heap_slots() <= 8192,
                "{} slots at {threads} threads",
                s.heap_slots()
            );
        }
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
            off: 0,
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
        let h = fb.reg(RegTy::Handle);
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
        let h = load_handle(&mut fb, AddrSpace::Frame, me);
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
        let obj = load_handle(&mut fb, AddrSpace::Shared, 0);
        let args = fb.regs(&[]);
        fb.emit(Inst::CallProt {
            func: incr,
            obj,
            args,
            dst: None,
        });
        stmt(&mut fb, StmtKind::Report);
        let obj = load_handle(&mut fb, AddrSpace::Shared, 0);
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

#[path = "common/gc_roots.rs"]
mod gc_roots;

/// Collections inside an activation use precise roots (#422): objects whose handles live
/// only in a register (of the top frame and of a caller), a temporary, a caller's frame
/// slot, a record in the frame and an array object, and one reached only through an
/// address in a register, survive; an object that only an
/// unrooted integer slot names is collected (no conservative retention). Same at 1 and 4
/// threads.
#[test]
fn precise_roots_inside_an_activation() {
    let (prog, p) = gc_roots::program();
    let prog = std::sync::Arc::new(prog);
    for threads in [1, 4] {
        let (r, vals, slots, live) = gc_roots::run(prog.clone(), p, None, threads);
        assert_eq!(vals, gc_roots::WANT, "{threads} threads {r:?}");
        let e = r.unwrap_err();
        assert!(
            matches!(&e.kind, ErrorKind::Runtime(m) if m.contains("deallocated")),
            "{e:?}"
        );
        // One slot per object without collections inside the activation.
        gc_roots::assert_churn_collected(slots, live);
    }
}

/// The runtime counts the processes made of each process function (`NativeFlags::instances`):
/// the compiled tier compiles a clocked process only when many share its code (#510).
#[test]
fn process_instances_are_counted() {
    let mut pb = ProgramBuilder::new();
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        wait_forever(&mut fb);
        fb.finish();
    }
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    assert_eq!(rt.native_flags().instances(p), 0);
    for _ in 0..3 {
        let inst = std::sync::Arc::new(yavs_interp::InstData::default());
        yavs_interp::InterpProcess::new(rt.clone(), p, inst, vec![], None).unwrap();
    }
    assert_eq!(rt.native_flags().instances(p), 3);
}

/// A 4-byte load sign-extends wherever the cell is (`Width::B4`, #746): instance data at a
/// constant offset (`Op::LoadInst`) and a frame slot (`Op::LoadSlot`) alike.
#[test]
fn four_byte_loads_sign_extend_from_instance_data_and_frame() {
    let mut pb = ProgramBuilder::new();
    let from_inst = pb.declare("from_inst", FuncKind::Function, None);
    let from_frame = pb.declare("from_frame", FuncKind::Function, None);
    {
        let mut fb = pb.define(from_inst);
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Inst, 8, Width::B4);
        fb.emit(Inst::Ret { v: Some(v) });
        fb.finish();
    }
    {
        let mut fb = pb.define(from_frame);
        let x = fb.param("x", Width::B4, ParamClass::Value, Mode::In, None);
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B4);
        fb.emit(Inst::Ret { v: Some(v) });
        fb.finish();
    }
    let prog = std::sync::Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let mut data = vec![0u8; 16];
    data[8..12].copy_from_slice(&(-3i32).to_le_bytes());
    let inst = std::sync::Arc::new(yavs_interp::InstData {
        data,
        ..Default::default()
    });
    let v = yavs_interp::call(rt.clone(), from_inst, inst.clone(), &[]).unwrap();
    assert_eq!(v.map(|v| v as i64), Some(-3));
    let v = yavs_interp::call(rt, from_frame, inst, &[(-7i64) as u64]).unwrap();
    assert_eq!(v.map(|v| v as i64), Some(-7));
}
