//! The LLVM tier against the interpreter (rule 5): hand-written IR run both ways.
#![cfg(feature = "llvm")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime, Tier, TierHost};
use yavs_ir::*;
use yavs_llvm::{AutoTier, Heat, LlvmTier, Note, OptLevel};

/// Counts the entries into compiled code.
struct Counting(LlvmTier, AtomicUsize);

impl Tier for Counting {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let r = self.0.run(func, pc, host);
        if r.is_some() {
            self.1.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
}

/// A program compiled once, called in the interpreter and with the LLVM tier.
struct Both {
    prog: Arc<Program>,
    tier: Arc<Counting>,
    inst: Arc<InstData>,
}

impl Both {
    fn new(prog: Program) -> Both {
        let tier = yavs_llvm::compile(&prog, OptLevel::O2, usize::MAX).unwrap();
        assert!(tier.notes.is_empty(), "{:?}", tier.notes);
        Both {
            prog: Arc::new(prog),
            tier: Arc::new(Counting(tier, AtomicUsize::new(0))),
            inst: Arc::new(InstData {
                data: (1..=16).collect(),
                ..InstData::default()
            }),
        }
    }

    fn call(&self, f: FuncId, args: &[u64], llvm: bool) -> Result<u64, String> {
        let rt = Runtime::new(self.prog.clone(), Bindings::default()).unwrap();
        if llvm {
            assert!(rt.set_tier(self.tier.clone()));
        }
        yavs_interp::call(rt, f, self.inst.clone(), args)
            .map(|v| v.expect("a value"))
            .map_err(|e| e.to_string())
    }

    /// Both results (value or runtime error text) and the entries into compiled code.
    fn both(&self, f: FuncId, args: &[u64]) -> (Result<u64, String>, Result<u64, String>, usize) {
        self.tier.1.store(0, Ordering::Relaxed);
        let (a, b) = (self.call(f, args, false), self.call(f, args, true));
        (a, b, self.tier.1.load(Ordering::Relaxed))
    }
}

fn stmt(fb: &mut FuncBuilder<'_>) {
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
}

/// `function f(a, b) return a op b` with an optional range check of the result.
fn binop(op: IntOp, range: Option<(i64, i64)>) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let msg = pb.check_msg("result", Dir::To);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    stmt(&mut fb);
    let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let (a, b) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: a,
        addr: pa,
        w: Width::B8,
    });
    fb.emit(Inst::Load {
        dst: b,
        addr: pbr,
        w: Width::B8,
    });
    let dst = fb.reg(RegTy::I64);
    match range {
        None => fb.emit(Inst::Bin { op, dst, a, b }),
        Some((l, h)) => {
            let (lo, hi) = (fb.int(l), fb.int(h));
            fb.emit(Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what: msg,
            });
        }
    }
    fb.emit(Inst::Ret { v: Some(dst) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn integer_ops_match_the_interpreter() {
    let vals = [
        0,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        1000,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
    ];
    let ops = [
        IntOp::Add,
        IntOp::Sub,
        IntOp::Mul,
        IntOp::Div,
        IntOp::Rem,
        IntOp::Mod,
        IntOp::Pow,
    ];
    for op in ops {
        for range in [None, Some((-100, 100))] {
            let (prog, f) = binop(op, range);
            let p = Both::new(prog);
            for &x in &vals {
                for &y in &vals {
                    let (a, b, n) = p.both(f, &[x as u64, y as u64]);
                    assert_eq!(a, b, "{op:?} {range:?} {x} {y}");
                    assert!(n > 0, "compiled code never ran");
                }
            }
        }
    }
}

#[test]
fn loop_runs_compiled() {
    let (prog, f) = sum();
    let p = Both::new(prog);
    let (a, b, n) = p.both(f, &[100_000]);
    assert_eq!(a, Ok(5_000_050_000));
    assert_eq!(a, b);
    // The whole loop runs in compiled code and the interpreter only executes `Ret`: one entry
    // from the function's start, or two when its first instruction is a definition the frame
    // starts with (skipped, so the interpreter runs the preamble and enters at the loop).
    assert!((1..=2).contains(&n), "{n} entries");
}

/// A threshold the loops of these tests pass early: seen in one sample of the interpreter.
const HOT: Heat = Heat {
    base: 1,
    per_instr: 0,
    period: 100,
    budget: 0,
};

/// `--tier=auto`: the loop becomes hot while it runs in the interpreter, is compiled on the
/// background thread and swapped in; the results never change (rule 5).
#[test]
fn auto_tier_compiles_hot_functions() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, _, why| seen.lock().unwrap().push((f, why)));
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, HOT, note, None).unwrap();
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    // The loop runs until the sampler has seen it and its code is compiled.
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert_eq!(call().unwrap(), Some(5_000_050_000));
    // The policy's clock is the instructions of the simulation threads, or the platform says why not.
    if let Some(why) = tier.clock_note() {
        assert!(
            why.contains("perf_event_open") || why.contains("no instruction counter"),
            "{why}"
        );
    }
    tier.stop(true);
    // A note per module: its native-frame code and its frame-mode code (#510).
    let notes = notes.lock().unwrap();
    assert!(!notes.is_empty() && notes.len() <= 2, "{notes:?}");
    assert!(notes.iter().all(|n| n.0 == f && n.1.is_ok()), "{notes:?}");
}

/// With a budget the compile thread waits until compiling costs at most a few percent of the
/// time it saw the interpreter run; a long run compiles, and the results never change. The
/// decision itself is tested deterministically (`within_budget` in `auto.rs`): no wall-clock
/// limit here, a loaded runner may be slow either way.
#[test]
fn auto_tier_compiles_within_its_budget() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let heat = Heat { budget: 2, ..HOT };
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, heat, note, None).unwrap();
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert_eq!(call().unwrap(), Some(5_000_050_000));
    tier.stop(true);
}

/// Ending the tier does not wait for the sampler to wake: a sampling period of half a minute
/// costs a stop right after the start, and one after the sampler began to sleep, no time at all
/// (a short run ended up to a period late).
#[test]
fn auto_tier_stops_without_waiting_for_the_sampler() {
    for wait in [Duration::ZERO, Duration::from_millis(300)] {
        let (prog, _) = sum();
        let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
        let note: Note = Box::new(|_, _, _| {});
        let heat = Heat {
            period: 30_000_000,
            ..Heat::DEFAULT
        };
        let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, heat, note, None).unwrap();
        std::thread::sleep(wait);
        let t0 = Instant::now();
        tier.stop(true);
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "stop waited for the sampler (after {wait:?})"
        );
    }
}

/// A deferred tier has no compile thread until `launch`: a hot loop stays interpreted, then it is
/// compiled once the thread runs; ending the tier before any launch is fine, and a launch after
/// the end does nothing (a run of a few milliseconds starts no thread, #426).
#[test]
fn auto_tier_deferred_starts_its_thread_at_launch() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let tier = AutoTier::start_deferred(&rt, OptLevel::O2, usize::MAX, HOT, note, None).unwrap();
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(300) {
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert_eq!(tier.compiled(), 0, "compiled with no thread");
    tier.launch().unwrap();
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert!(tier.stop(true));

    let (prog, _) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let tier = AutoTier::start_deferred(&rt, OptLevel::O2, usize::MAX, HOT, note, None).unwrap();
    assert!(tier.stop(false));
    tier.launch().unwrap();
    assert!(tier.stop(true));
}

/// One call that never returns to the kernel (a process that runs its loop to the end without
/// suspending) is seen by the sampler only, and the big loop is still compiled within the
/// default budget and swapped in while it runs (#432). The budget decision itself is tested
/// deterministically in `auto.rs`; this checks the whole path.
///
/// What the sampler credits to a function is the work it saw, so the compile is decided after a
/// number of calls and not after a time (#755, #777): the calls go on until it is, each one's
/// result checked. The payback rule asks for about half of one call's work, so the first call
/// decides it on a quiet host; a host that credits less takes more calls.
#[test]
fn auto_tier_compiles_a_long_loop_of_one_call() {
    const MAX_CALLS: u32 = 40;
    let (prog, f) = sum_padded(800);
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let heat = Heat { budget: 2, ..HOT };
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, heat, note, None).unwrap();
    let n = 3_000_000u64;
    let mut calls = 0;
    while tier.compiled() == 0 {
        calls += 1;
        assert!(calls <= MAX_CALLS, "the loop was never compiled");
        let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[n]);
        assert_eq!(r.unwrap(), Some(n * (n + 1) / 2));
    }
    assert_eq!(tier.compiled(), 1);
    tier.stop(true);
}

/// A function compiled at O0 that stays hot in its compiled code is compiled again at O2 and
/// swapped in; the results never change.
#[test]
fn auto_tier_recompiles_hot_compiled_code_at_o2() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, level, why| seen.lock().unwrap().push((f, level, why)));
    let tier = AutoTier::start(&rt, OptLevel::O0, usize::MAX, HOT, note, None).unwrap();
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    let t0 = Instant::now();
    while !notes.lock().unwrap().iter().any(|n| n.1 == OptLevel::O2) {
        assert!(t0.elapsed() < Duration::from_secs(60), "never recompiled");
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert_eq!(call().unwrap(), Some(5_000_050_000));
    tier.stop(true);
    let notes = notes.lock().unwrap();
    let mut levels: Vec<_> = notes.iter().map(|n| (n.0, n.1, n.2.is_ok())).collect();
    // A note per module (native-frame and frame-mode code, #510).
    levels.dedup();
    assert_eq!(levels, [(f, OptLevel::O0, true), (f, OptLevel::O2, true)]);
}

/// A function of `COMPUTE_MAX` IR instructions or more is compiled again at `FastRa` (not O2) when
/// it stays hot, and swapped in; the results never change (#715).
#[test]
fn auto_tier_recompiles_a_big_function_at_fast_ra() {
    let (prog, f) = sum_padded(1_600);
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, level, why| seen.lock().unwrap().push((f, level, why)));
    let tier = AutoTier::start(&rt, OptLevel::O0, usize::MAX, HOT, note, None).unwrap();
    let n = 20_000u64;
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[n]);
    let t0 = Instant::now();
    while !notes
        .lock()
        .unwrap()
        .iter()
        .any(|x| x.1 == OptLevel::FastRa)
    {
        assert!(t0.elapsed() < Duration::from_secs(120), "never recompiled");
        assert_eq!(call().unwrap(), Some(n * (n + 1) / 2));
    }
    assert_eq!(call().unwrap(), Some(n * (n + 1) / 2));
    tier.stop(true);
    let notes = notes.lock().unwrap();
    let levels: Vec<_> = notes.iter().map(|n| (n.1, n.2.is_ok())).collect();
    assert!(levels.contains(&(OptLevel::FastRa, true)), "{levels:?}");
    assert!(!levels.iter().any(|l| l.0 == OptLevel::O2), "{levels:?}");
}

/// A short function without a loop that runs on native frames is compiled (at O2) when hot,
/// for interpreted callers (`Tier::call`) and with a compiled loop that calls it, which would
/// otherwise leave its compiled code at every call.
#[test]
fn auto_tier_compiles_short_callees_of_compiled_code() {
    let (prog, run, _) = calls();
    let inc = prog
        .funcs
        .iter()
        .map(|(id, _)| id)
        .find(|&id| id != run)
        .unwrap();
    assert_eq!(prog.func(inc).unwrap().name, "inc");
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, level, why| seen.lock().unwrap().push((f, level, why)));
    // First tier O2: the callee compiles with its caller, at O2, in one step. (At O0 it would
    // reach O2 only when a sample happens to find the interpreter inside it, which on a slow
    // host can take minutes.)
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, HOT, note, None).unwrap();
    let call = || yavs_interp::call(rt.clone(), run, Arc::new(InstData::default()), &[10_000]);
    let t0 = Instant::now();
    let done = || {
        notes
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.0 == inc && n.1 == OptLevel::O2 && n.2.is_ok())
    };
    while !done() {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        assert_eq!(call().unwrap(), Some(10_000));
    }
    assert_eq!(call().unwrap(), Some(10_000));
    tier.stop(true);
}

/// A hot function over `--tier-max-size` stays interpreted, with a note.
#[test]
fn auto_tier_skips_functions_over_the_size_cap() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, _, why| seen.lock().unwrap().push((f, why)));
    let tier = AutoTier::start(&rt, OptLevel::O2, 3, HOT, note, None).unwrap();
    let t0 = Instant::now();
    while notes.lock().unwrap().is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(60), "no note");
        let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
        assert_eq!(r.unwrap(), Some(5_000_050_000));
    }
    tier.stop(true);
    assert_eq!(tier.compiled(), 0);
    let notes = notes.lock().unwrap();
    assert_eq!(notes.len(), 1);
    let why = notes[0].1.as_ref().unwrap_err();
    assert!(
        why.contains("over the size cap of 3 (--tier-max-size)"),
        "{why}"
    );
}

/// sum := 0; for i in 0 to n loop sum := sum + i; end loop; return sum;
fn sum() -> (Program, FuncId) {
    sum_padded(0)
}

/// `sum` with `pad` additions of zero in the loop body: a big function for the budget.
fn sum_padded(pad: usize) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("sum", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    stmt(&mut fb);
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: n,
        addr: pn,
        w: Width::B8,
    });
    let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Le,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    let zero = fb.int(0);
    for _ in 0..pad {
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: acc,
            a: acc,
            b: zero,
        });
    }
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: acc,
        a: acc,
        b: i,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// `function f(a, b) return <body>` where `body` builds the result from the two argument
/// registers, with a 32-byte frame area `local` and a 16-byte constant `c0` (1 to 16).
fn two(body: impl FnOnce(&mut FuncBuilder<'_>, Reg, Reg, Reg) -> Reg) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    pb.konst(ConstData::Bytes((1..=16).collect()));
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    let local = fb.slot(32, 8);
    stmt(&mut fb);
    let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let (a, b) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    for (dst, addr) in [(a, pa), (b, pbr)] {
        fb.emit(Inst::Load {
            dst,
            addr,
            w: Width::B8,
        });
    }
    let l = fb.addr(AddrSpace::Frame, local);
    let v = body(&mut fb, a, b, l);
    fb.emit(Inst::Ret { v: Some(v) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

fn index(fb: &mut FuncBuilder<'_>, base: Reg, pos: Reg) -> Reg {
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst,
        base,
        pos,
        stride: 1,
    });
    dst
}

fn alloc(fb: &mut FuncBuilder<'_>, size: Reg) -> Reg {
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst, size });
    dst
}

fn load(fb: &mut FuncBuilder<'_>, addr: Reg, w: Width) -> Reg {
    let dst = fb.reg(if w == Width::Sig {
        RegTy::Sig
    } else {
        RegTy::I64
    });
    fb.emit(Inst::Load { dst, addr, w });
    dst
}

/// A descriptor `left, right, dir` in temporary memory.
fn dope(fb: &mut FuncBuilder<'_>, l: Reg, r: Reg, dir: Reg) -> Reg {
    let n = fb.int(24);
    let at = alloc(fb, n);
    let parts = fb.regs(&[l, r, dir]);
    fb.emit(Inst::MakeDope { at, parts });
    at
}

const EDGE: [i64; 14] = [
    0,
    1,
    2,
    7,
    8,
    15,
    16,
    24,
    31,
    32,
    -1,
    i64::MAX,
    i64::MIN,
    1 << 32,
];

/// Every pair of edge values gives the same result or error text both ways, compiled code
/// runs, and none of `native` is left to the interpreter.
fn check(prog: Program, f: FuncId, native: &[&str], args: &[i64]) {
    let p = Both::new(prog);
    for k in native {
        assert!(
            !p.tier.0.interpreted.iter().any(|(n, _)| n == k),
            "{k} interpreted: {:?}",
            p.tier.0.interpreted
        );
    }
    for &x in args {
        for &y in args {
            let (a, b, n) = p.both(f, &[x as u64, y as u64]);
            assert_eq!(a, b, "{native:?} {x} {y}");
            assert!(n > 0, "compiled code never ran");
        }
    }
}

#[test]
fn loads_and_stores_match_the_interpreter() {
    for w in [Width::B1, Width::B4, Width::B8] {
        // local[b] := a (width w); return local[b]
        let (prog, f) = two(|fb, a, b, l| {
            let at = index(fb, l, b);
            fb.emit(Inst::Store {
                addr: at,
                src: a,
                w,
            });
            load(fb, at, w)
        });
        check(prog, f, &["Load", "Store", "AddrOf"], &EDGE);
    }
    // A signal reference pair (a, b) loaded from local[0] and stored at local[b]; returns
    // the second half read back.
    let (prog, f) = two(|fb, a, b, l| {
        let parts = fb.regs(&[a, b]);
        fb.emit(Inst::MakeDope { at: l, parts });
        let s = load(fb, l, Width::Sig);
        let at = index(fb, l, b);
        fb.emit(Inst::Store {
            addr: at,
            src: s,
            w: Width::Sig,
        });
        let eight = fb.int(8);
        let hi = index(fb, at, eight);
        load(fb, hi, Width::B8)
    });
    check(prog, f, &["Load", "Store"], &EDGE);
    // Reads of the instance data and a constant (through the object cache) at a + b.
    for space in [AddrSpace::Inst, AddrSpace::Const(ConstId(0))] {
        for w in [Width::B1, Width::B4, Width::B8] {
            let (prog, f) = two(|fb, a, b, _| {
                let base = fb.addr(space, 0);
                let at = index(fb, base, a);
                let at = index(fb, at, b);
                load(fb, at, w)
            });
            check(prog, f, &["Load", "AddrOf"], &EDGE);
        }
    }
    // A write to read-only memory is the interpreter's error.
    let (prog, f) = two(|fb, a, b, _| {
        let base = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        let at = index(fb, base, b);
        fb.emit(Inst::Store {
            addr: at,
            src: a,
            w: Width::B8,
        });
        a
    });
    check(prog, f, &["Store"], &[0, 1, 9]);
}

#[test]
fn temporaries_and_copies_match_the_interpreter() {
    // Two allocations: the second one's address, and its contents after a store of b.
    for addr in [false, true] {
        let (prog, f) = two(|fb, a, b, _| {
            let _ = alloc(fb, a);
            let eight = fb.int(8);
            let t = alloc(fb, eight);
            fb.emit(Inst::Store {
                addr: t,
                src: b,
                w: Width::B4,
            });
            if addr { t } else { load(fb, t, Width::B8) }
        });
        check(prog, f, &["Alloc", "Stmt"], &EDGE);
    }
    // A loop of statements each allocating a bytes: the area is reset at every `Stmt`.
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let _ = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, done) = (fb.block(), fb.block());
    stmt(&mut fb);
    let pa = fb.addr(AddrSpace::Frame, sa);
    let a = load(&mut fb, pa, Width::B8);
    let (i, one, n) = (fb.int(0), fb.int(1), fb.int(1000));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    stmt(&mut fb);
    let t = alloc(&mut fb, a);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Lt,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: head,
        f: done,
    });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(t) });
    fb.finish();
    check(
        pb.finish().unwrap(),
        f,
        &["Alloc", "Stmt"],
        &[0, 1, 13, 1 << 20, 1 << 32, -1],
    );
    // local[0..32] := 1, 2, ...; copy b bytes from local[a] to local[0]; return local[0].
    let (prog, f) = two(|fb, a, b, l| {
        let ws: Vec<Reg> = (0..4).map(|k| fb.int(0x0807_0605_0403_0201 + k)).collect();
        let parts = fb.regs(&ws);
        fb.emit(Inst::MakeDope { at: l, parts });
        let src = index(fb, l, a);
        fb.emit(Inst::Copy {
            dst: l,
            src,
            bytes: b,
        });
        load(fb, l, Width::B8)
    });
    check(prog, f, &["Copy", "MakeDope"], &EDGE);
    // A constant copied into temporary memory.
    let (prog, f) = two(|fb, a, b, _| {
        let t = alloc(fb, a);
        let c = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        fb.emit(Inst::Copy {
            dst: t,
            src: c,
            bytes: b,
        });
        load(fb, t, Width::B4)
    });
    check(prog, f, &["Copy"], &EDGE);
}

#[test]
fn descriptors_match_the_interpreter() {
    for dir in [0, 1, 2] {
        // 'length of (a, b, dir), and the position of 5 in it.
        let (prog, f) = two(|fb, a, b, _| {
            let d = fb.int(dir);
            let at = dope(fb, a, b, d);
            let n = fb.reg(RegTy::I64);
            fb.emit(Inst::DopeLen {
                dst: n,
                dope: at,
                dim: 0,
            });
            n
        });
        check(prog, f, &["DopeLen", "MakeDope"], &EDGE);
        for idx in [0, 5, -3] {
            let (prog, f) = two(|fb, a, b, _| {
                let d = fb.int(dir);
                let at = dope(fb, a, b, d);
                let i = fb.int(idx);
                let what = fb.program().check_msg("v", Dir::To);
                let dst = fb.reg(RegTy::I64);
                fb.emit(Inst::Pos {
                    dst,
                    dope: at,
                    dim: 0,
                    idx: i,
                    what,
                });
                dst
            });
            check(prog, f, &["Pos"], &EDGE);
        }
    }
    // (a to b) against (b downto 3) and (0 to 4), two dimensions.
    let (prog, f) = two(|fb, a, b, _| {
        let (z, one, three, four) = (fb.int(0), fb.int(1), fb.int(3), fb.int(4));
        let n = fb.int(48);
        let x = alloc(fb, n);
        let parts = fb.regs(&[a, b, z, b, three, one]);
        fb.emit(Inst::MakeDope { at: x, parts });
        let y = alloc(fb, n);
        let parts = fb.regs(&[z, four, z, a, b, z]);
        fb.emit(Inst::MakeDope { at: y, parts });
        let what = fb.program().check_msg("v", Dir::To);
        fb.emit(Inst::SubtypeCheck {
            dope: x,
            want: y,
            dims: 2,
            what,
        });
        a
    });
    check(prog, f, &["SubtypeCheck"], &EDGE);
    // Field 1 of a layout [size, align, 0, a] at local[b].
    let (prog, f) = two(|fb, a, b, l| {
        let (z, eight) = (fb.int(0), fb.int(8));
        let n = fb.int(32);
        let lay = alloc(fb, n);
        let parts = fb.regs(&[eight, eight, z, a]);
        fb.emit(Inst::MakeDope { at: lay, parts });
        let base = index(fb, l, b);
        let dst = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Field {
            dst,
            base,
            layout: lay,
            n: 1,
        });
        dst
    });
    check(prog, f, &["Field"], &EDGE);
}

#[test]
fn logic_elements_match_the_interpreter() {
    // local is two bit-plane blocks of 4 x 64 bits: element b := code a, then read back
    // element b and element b xor 1 (untouched neighbours stay).
    for other in [false, true] {
        let (prog, f) = two(|fb, a, b, l| {
            fb.emit(Inst::LSet {
                base: l,
                idx: b,
                v: a,
            });
            let i = if other {
                let one = fb.int(1);
                let i = fb.reg(RegTy::I64);
                fb.emit(Inst::Bin {
                    op: IntOp::Add,
                    dst: i,
                    a: b,
                    b: one,
                });
                i
            } else {
                b
            };
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::LGet {
                dst,
                base: l,
                idx: i,
            });
            dst
        });
        check(
            prog,
            f,
            &["LGet", "LSet"],
            &[0, 1, 3, 8, 9, 63, 64, 127, 128, -1, 1 << 40],
        );
    }
    // Elements of a constant (read only) and of temporary memory.
    let (prog, f) = two(|fb, a, b, _| {
        let c = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::LGet {
            dst: x,
            base: c,
            idx: a,
        });
        let n = fb.int(64);
        let t = alloc(fb, n);
        fb.emit(Inst::LSet {
            base: t,
            idx: b,
            v: x,
        });
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::LGet {
            dst,
            base: t,
            idx: b,
        });
        dst
    });
    check(prog, f, &["LGet", "LSet"], &[0, 1, 5, 63, 64, 127, 128, -1]);
}

/// `LCopy` of `n` plane elements from offset `b` to offset `a` within ten blocks of temporary
/// memory (overlapping) or from a constant into them, at every edge of a block and beyond
/// the eight blocks compiled code copies itself: the same bytes (a weighted sum of all 32-bit
/// words) or the same error text.
#[test]
fn logic_copies_match_the_interpreter() {
    const LEN: usize = 320;
    let pattern: Vec<u8> = (0..LEN)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3])
        .collect();
    for n in [0, 1, 7, 63, 64, 65, 128, 190, 300, 449, 500] {
        for from_const in [false, true] {
            let mut pb = ProgramBuilder::new();
            let k = pb.konst(ConstData::Bytes(pattern.clone()));
            let f = pb.declare("f", FuncKind::Function, None);
            let mut fb = pb.define(f);
            let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
            let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
            stmt(&mut fb);
            let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
            let (a, b) = (load(&mut fb, pa, Width::B8), load(&mut fb, pbr, Width::B8));
            let c = fb.addr(AddrSpace::Const(k), 0);
            let size = fb.int(LEN as i64);
            let t = alloc(&mut fb, size);
            fb.emit(Inst::Copy {
                dst: t,
                src: c,
                bytes: size,
            });
            let n = fb.int(n);
            fb.emit(Inst::LCopy {
                dst: t,
                dst_off: a,
                src: if from_const { c } else { t },
                src_off: b,
                n,
            });
            let mut acc = fb.int(0);
            for w in 0..LEN / 4 {
                let at = fb.int(w as i64 * 4);
                let p = index(&mut fb, t, at);
                let v = load(&mut fb, p, Width::B4);
                let m = fb.int(w as i64 + 1);
                let (x, sum) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
                fb.emit(Inst::Bin {
                    op: IntOp::Mul,
                    dst: x,
                    a: v,
                    b: m,
                });
                fb.emit(Inst::Bin {
                    op: IntOp::Add,
                    dst: sum,
                    a: acc,
                    b: x,
                });
                acc = sum;
            }
            fb.emit(Inst::Ret { v: Some(acc) });
            fb.finish();
            check(
                pb.finish().unwrap(),
                f,
                &["LCopy"],
                &[0, 1, 5, 63, 64, 65, 127, 128, 200, 639, -1, 1 << 40],
            );
        }
    }
}

/// `LCopy` with constant offsets and count (the inline form for a range inside one plane block
/// on each side, also over the same bytes and from a constant): the same bytes as the
/// interpreter, a weighted sum of all 32-bit words.
#[test]
fn constant_logic_copies_match_the_interpreter() {
    const LEN: usize = 320;
    let pattern: Vec<u8> = (0..LEN)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3])
        .collect();
    let shapes = [
        (1, 0, 0),
        (1, 63, 0),
        (7, 3, 3),
        (7, 5, 9),
        (32, 0, 0),
        (32, 31, 31),
        (32, 1, 40),
        (33, 60, 0),
        (63, 1, 0),
        (64, 0, 0),
        (64, 64, 128),
        (64, 70, 100),
        (30, 127, 127),
        (30, 100, 99),
        (40, 5, 6),
        (200, 0, 0),
    ];
    for (n, a, b) in shapes {
        for from_const in [false, true] {
            let mut pb = ProgramBuilder::new();
            let k = pb.konst(ConstData::Bytes(pattern.clone()));
            let f = pb.declare("f", FuncKind::Function, None);
            let mut fb = pb.define(f);
            fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
            fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
            stmt(&mut fb);
            let c = fb.addr(AddrSpace::Const(k), 0);
            let size = fb.int(LEN as i64);
            let t = alloc(&mut fb, size);
            fb.emit(Inst::Copy {
                dst: t,
                src: c,
                bytes: size,
            });
            let (a, b, n) = (fb.int(a), fb.int(b), fb.int(n));
            fb.emit(Inst::LCopy {
                dst: t,
                dst_off: a,
                src: if from_const { c } else { t },
                src_off: b,
                n,
            });
            let mut acc = fb.int(0);
            for w in 0..LEN / 4 {
                let at = fb.int(w as i64 * 4);
                let p = index(&mut fb, t, at);
                let v = load(&mut fb, p, Width::B4);
                let m = fb.int(w as i64 + 1);
                let (x, sum) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
                fb.emit(Inst::Bin {
                    op: IntOp::Mul,
                    dst: x,
                    a: v,
                    b: m,
                });
                fb.emit(Inst::Bin {
                    op: IntOp::Add,
                    dst: sum,
                    a: acc,
                    b: x,
                });
                acc = sum;
            }
            fb.emit(Inst::Ret { v: Some(acc) });
            fb.finish();
            check(pb.finish().unwrap(), f, &["LCopy"], &[0]);
        }
    }
}

/// The scalar `std_ulogic` natives compiled code looks up in a table give what the natives
/// give for all nine codes of each operand, and the native's error beyond them.
#[test]
fn std_ulogic_natives_match_the_interpreter() {
    let gates = ["and", "nand", "or", "nor", "xor", "xnor"].map(|g| {
        (
            format!("yavs.ieee.std_logic_1164.\"{g}\"(std_ulogic,std_ulogic)std_ulogic"),
            2,
        )
    });
    let ones = ["not", "to_x01", "to_x01z", "to_ux01"].map(|n| {
        (
            format!("yavs.ieee.std_logic_1164.\"{n}\"(std_ulogic)std_ulogic"),
            1,
        )
    });
    let truthy = (
        "yavs.ieee.std_logic_1164.\"??\"(std_ulogic)boolean".to_string(),
        1,
    );
    for (sym, n) in gates.into_iter().chain(ones).chain([truthy]) {
        assert!(yavs_interp::code_table(&sym).is_some(), "{sym}");
        let mut pb = ProgramBuilder::new();
        let native = pb.native(&sym, None);
        let f = pb.declare("f", FuncKind::Function, None);
        let mut fb = pb.define(f);
        let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
        let (a, b) = (load(&mut fb, pa, Width::B8), load(&mut fb, pbr, Width::B8));
        let args = fb.regs(&[a, b][..n]);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: Some(dst),
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
        check(
            pb.finish().unwrap(),
            f,
            &["CallNative"],
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1],
        );
    }
}

#[path = "../../yavs-interp/tests/common/gc_roots.rs"]
mod gc_roots;

/// Compiled code reaches `new` through `TierHost::step` with the registers, frame slots and
/// temporaries in the machine, so a collection there sees the precise roots the interpreter
/// sees (#422): the same objects survive and the unrooted one is collected.
#[test]
fn precise_roots_under_compiled_code() {
    let (prog, p) = gc_roots::program();
    let both = Both::new(prog);
    let tier: Arc<dyn Tier> = both.tier.clone();
    let (r, vals, slots, live) = gc_roots::run(both.prog.clone(), p, Some(tier), 1);
    assert!(
        both.tier.1.load(Ordering::Relaxed) > 0,
        "compiled code never ran"
    );
    assert_eq!(vals, gc_roots::WANT, "{r:?}");
    let e = r.unwrap_err();
    assert!(e.to_string().contains("deallocated"), "{e}");
    gc_roots::assert_churn_collected(slots, live);
}

/// `inc(a) = a + 1` (range checked up to 1_000_000, two paths to its return, one of them keeping a frame address), `run(n)` calling it `n` times in a loop,
/// and `depth(n) = if n = 0 then 0 else depth(n - 1) + 1`.
fn calls() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let msg = pb.check_msg("result", Dir::To);
    let inc = pb.declare("inc", FuncKind::Function, None);
    let run = pb.declare("run", FuncKind::Function, None);
    let depth = pb.declare("depth", FuncKind::Function, None);
    let arg = |fb: &mut FuncBuilder<'_>| {
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(fb);
        let p = fb.addr(AddrSpace::Frame, s);
        load(fb, p, Width::B8)
    };
    {
        let mut fb = pb.define(inc);
        let a = arg(&mut fb);
        let (one, lo, hi) = (fb.int(1), fb.int(0), fb.int(1_000_000));
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::BinRange {
            op: IntOp::Add,
            dst,
            a,
            b: one,
            lo,
            hi,
            what: msg,
        });
        // Two ways to the same return.
        let (zero, c) = (fb.int(0), fb.reg(RegTy::I64));
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a,
            b: zero,
        });
        let (t, e) = (fb.block(), fb.block());
        fb.emit(Inst::CondBr { c, t, f: e });
        fb.at(t);
        // (Keeps the address of a slot of its frame: the optimiser copies a function with
        // branches into its caller, unless it does something with its frame.)
        let keep = fb.slot(8, 8);
        let at = fb.addr(AddrSpace::Frame, keep);
        fb.emit(Inst::Store {
            addr: at,
            src: at,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.at(e);
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
    }
    {
        let mut fb = pb.define(run);
        let n = arg(&mut fb);
        let (head, body, done) = (fb.block(), fb.block(), fb.block());
        let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: i,
            b: n,
        });
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb);
        let args = fb.regs(&[acc]);
        fb.emit(Inst::Call {
            func: inc,
            args,
            dst: Some(acc),
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(done);
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
    }
    {
        let mut fb = pb.define(depth);
        let n = arg(&mut fb);
        let (base, rec) = (fb.block(), fb.block());
        let (zero, one) = (fb.int(0), fb.int(1));
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: n,
            b: zero,
        });
        fb.emit(Inst::CondBr { c, t: base, f: rec });
        fb.at(base);
        fb.emit(Inst::Ret { v: Some(zero) });
        fb.at(rec);
        let m = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Sub,
            dst: m,
            a: n,
            b: one,
        });
        let args = fb.regs(&[m]);
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: depth,
            args,
            dst: Some(d),
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: d,
            a: d,
            b: one,
        });
        fb.emit(Inst::Ret { v: Some(d) });
        fb.finish();
    }
    (pb.finish().unwrap(), run, depth)
}

/// `boxed(a)` returns the address of a temporary holding `a + 1`; `sum(n)` adds the values of
/// `n` such calls, one statement each (the caller's temporaries end before every call).
fn temp_calls() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let boxed = pb.declare("boxed", FuncKind::Function, None);
    let run = pb.declare("sum", FuncKind::Function, None);
    {
        let mut fb = pb.define(boxed);
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let p = fb.addr(AddrSpace::Frame, s);
        let a = load(&mut fb, p, Width::B8);
        let one = fb.int(1);
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: v,
            a,
            b: one,
        });
        let n = fb.int(8);
        let t = alloc(&mut fb, n);
        fb.emit(Inst::Store {
            addr: t,
            src: v,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: Some(t) });
        fb.finish();
    }
    {
        let mut fb = pb.define(run);
        let s = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let p = fb.addr(AddrSpace::Frame, s);
        let n = load(&mut fb, p, Width::B8);
        let (head, body, done) = (fb.block(), fb.block(), fb.block());
        let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: i,
            b: n,
        });
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb);
        let args = fb.regs(&[i]);
        let t = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Call {
            func: boxed,
            args,
            dst: Some(t),
        });
        let v = load(&mut fb, t, Width::B8);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: acc,
            a: acc,
            b: v,
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(done);
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
    }
    (pb.finish().unwrap(), run)
}

/// A compiled callee returns a temporary of its own to a compiled caller whose statements
/// end their temporaries in compiled code: the interpreter sees the length compiled code left.
#[test]
fn temporaries_across_compiled_calls() {
    let (prog, run) = temp_calls();
    let p = Both::new(prog);
    let (a, b, _) = p.both(run, &[1000]);
    assert_eq!(a, Ok(500_500));
    assert_eq!(a, b);
}

/// Compiled code calls compiled code without returning to the interpreter (#336); an error in
/// the callee, a recursion deeper than the native stack allowance and the results are the
/// interpreter's.
#[test]
fn calls_between_compiled_functions() {
    let (prog, run, depth) = calls();
    let p = Both::new(prog);
    let (a, b, n) = p.both(run, &[100_000]);
    assert_eq!(a, Ok(100_000));
    assert_eq!(a, b);
    // The loop and the calls of `inc` run compiled: a few entries (the first calls may move
    // the register stack, which leaves the call to the interpreter), not one per call.
    assert!(n <= 3, "{n} entries");
    // `inc` fails in its 1_000_001st call, inside the compiled caller.
    let (a, b, _) = p.both(run, &[2_000_000]);
    assert!(a.as_ref().is_err_and(|e| e.contains("result")), "{a:?}");
    assert_eq!(a, b);
    let (a, b, n) = p.both(depth, &[5000]);
    assert_eq!(a, Ok(5000));
    assert_eq!(a, b);
    assert!(n > 1, "{n} entries: no recursion left to the interpreter");
    // Past the call depth limit: the native frames are materialised and the interpreter
    // reports the limit with the same text and trace.
    let (a, b, _) = p.both(depth, &[20_000]);
    assert!(a.as_ref().is_err_and(|e| e.contains("depth")), "{a:?}");
    assert_eq!(a, b);
}

/// All three functions can run on native frames (docs/decisions.md "Compiled frames"): `run`
/// calls `inc` and `depth` calls itself without leaving compiled code.
#[test]
fn native_frames() {
    let (prog, run, depth) = calls();
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 3);
    for (f, k) in [(run, 1000), (run, 10_000), (depth, 300)] {
        let (a, b, n) = p.both(f, &[k]);
        assert_eq!((a, b), (Ok(k), Ok(k)));
        // One entry into compiled code, at the first instruction after the preloaded
        // definitions, whatever the number of calls.
        assert_eq!(n, 1, "entries for {f} {k}");
    }
}

/// `deep(n)` recurses to `n = 0`; every level keeps `x = 3n` and `y = x + 7` across its call and
/// returns `deep(n - 1) + x - y`, so the result is `-7n` only if the registers a materialised
/// frame is resumed with are the ones that were live at the call.
fn live_across() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let deep = pb.declare("deep", FuncKind::Function, None);
    let mut fb = pb.define(deep);
    let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    stmt(&mut fb);
    let p = fb.addr(AddrSpace::Frame, s);
    let n = load(&mut fb, p, Width::B8);
    let (base, rec) = (fb.block(), fb.block());
    let (zero, one, three, seven) = (fb.int(0), fb.int(1), fb.int(3), fb.int(7));
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: n,
        b: zero,
    });
    fb.emit(Inst::CondBr { c, t: base, f: rec });
    fb.at(base);
    fb.emit(Inst::Ret { v: Some(zero) });
    fb.at(rec);
    let (m, x, y, d, r) = (
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
    );
    let bin = |fb: &mut FuncBuilder<'_>, op, dst, a, b| fb.emit(Inst::Bin { op, dst, a, b });
    bin(&mut fb, IntOp::Sub, m, n, one);
    bin(&mut fb, IntOp::Mul, x, n, three);
    bin(&mut fb, IntOp::Add, y, x, seven);
    let args = fb.regs(&[m]);
    fb.emit(Inst::Call {
        func: deep,
        args,
        dst: Some(d),
    });
    bin(&mut fb, IntOp::Add, r, d, x);
    bin(&mut fb, IntOp::Sub, r, r, y);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    (pb.finish().unwrap(), deep)
}

/// Native frames left to the interpreter (a recursion deeper than the native stack allowance, one
/// past the call depth limit) carry the registers live at their call, and only those.
#[test]
fn materialised_frames_keep_the_live_registers() {
    let (prog, deep) = live_across();
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 1);
    for n in [10u64, 5000, 9000] {
        let (a, b, _) = p.both(deep, &[n]);
        assert_eq!(a, Ok((-7 * n as i64) as u64), "{n}");
        assert_eq!(a, b, "{n}");
    }
    let (a, b, _) = p.both(deep, &[20_000]);
    assert!(a.as_ref().is_err_and(|e| e.contains("depth")), "{a:?}");
    assert_eq!(a, b);
}

/// `slots(n)` keeps `v := 2n` and a 4-byte `w := n` in slots of its frame (no address of them leaves
/// its own loads and stores, so the frame is not registered for callees) across its call of
/// `slots(n - 1)` and returns that call plus `v` plus `w`: `n (n + 1) + n (n + 1) / 2`. Frames the
/// native stack allowance leaves to the interpreter must carry the slots as the function stored
/// them.
fn closed_slots() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("slots", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let (vv, vw) = (fb.slot(8, 8), fb.slot(4, 4));
    stmt(&mut fb);
    let p = fb.addr(AddrSpace::Frame, s);
    let n = load(&mut fb, p, Width::B8);
    let (base, rec) = (fb.block(), fb.block());
    let (zero, one, two) = (fb.int(0), fb.int(1), fb.int(2));
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: n,
        b: zero,
    });
    fb.emit(Inst::CondBr { c, t: base, f: rec });
    fb.at(base);
    fb.emit(Inst::Ret { v: Some(zero) });
    fb.at(rec);
    let (pv, pw) = (fb.addr(AddrSpace::Frame, vv), fb.addr(AddrSpace::Frame, vw));
    let (v, m, d) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Bin {
        op: IntOp::Mul,
        dst: v,
        a: n,
        b: two,
    });
    fb.emit(Inst::Store {
        addr: pv,
        src: v,
        w: Width::B8,
    });
    fb.emit(Inst::Store {
        addr: pw,
        src: n,
        w: Width::B4,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: m,
        a: n,
        b: one,
    });
    let args = fb.regs(&[m]);
    fb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(d),
    });
    let (v2, w2, r1, r2) = (
        load(&mut fb, pv, Width::B8),
        load(&mut fb, pw, Width::B4),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: r1,
        a: d,
        b: v2,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: r2,
        a: r1,
        b: w2,
    });
    fb.emit(Inst::Ret { v: Some(r2) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// `byref(k)`: `v := k`, then a call of `bump(&v)`, which adds 1 through its pointer parameter
/// (the callee's own frame is closed, the caller's is not); returns `v`.
fn by_reference() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let bump = pb.declare("bump", FuncKind::Procedure, None);
    let byref = pb.declare("byref", FuncKind::Function, None);
    {
        let mut fb = pb.define(bump);
        let s = fb.param("p", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let pp = fb.addr(AddrSpace::Frame, s);
        let p = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: p,
            addr: pp,
            w: Width::B8,
        });
        let v = load(&mut fb, p, Width::B8);
        let one = fb.int(1);
        let w = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: w,
            a: v,
            b: one,
        });
        fb.emit(Inst::Store {
            addr: p,
            src: w,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let mut fb = pb.define(byref);
    let s = fb.param("k", Width::B8, ParamClass::Value, Mode::In, None);
    let var = fb.slot(8, 8);
    stmt(&mut fb);
    let pk = fb.addr(AddrSpace::Frame, s);
    let k = load(&mut fb, pk, Width::B8);
    let pv = fb.addr(AddrSpace::Frame, var);
    fb.emit(Inst::Store {
        addr: pv,
        src: k,
        w: Width::B8,
    });
    let args = fb.regs(&[pv]);
    fb.emit(Inst::Call {
        func: bump,
        args,
        dst: None,
    });
    let r = load(&mut fb, pv, Width::B8);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    (pb.finish().unwrap(), byref, bump)
}

/// A closed callee reaches the slots of its native caller through the pointer it was given.
#[test]
fn closed_callee_writes_through_a_pointer_to_its_caller() {
    let (prog, byref, _) = by_reference();
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 2);
    for k in [0u64, 41] {
        let (a, b, _) = p.both(byref, &[k]);
        assert_eq!(a, Ok(k + 1));
        assert_eq!(a, b);
    }
}

/// Slots of a frame no address of leaves are kept by the compiled code and handed to the
/// interpreter when a frame is materialised (a recursion deeper than the native stack allowance).
#[test]
fn closed_frames_hand_their_slots_to_the_interpreter() {
    let (prog, f) = closed_slots();
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 1);
    for n in [10u64, 5000, 9000] {
        let want = n * (n + 1) + n * (n + 1) / 2;
        let (a, b, _) = p.both(f, &[n]);
        assert_eq!(a, Ok(want), "{n}");
        assert_eq!(a, b, "{n}");
    }
}

/// `vars(a, b)`: variables of the frame (a 64-bit, a 32-bit and an 8-bit one, each loaded and
/// stored through an address of its own as the lowering does, and a fourth one whose address
/// a `MakeDope` writes through) in a loop of `b mod 8` rounds with a call in it. Compiled code
/// keeps the variables as values (`regs::Scalars`) and must write them back where the
/// interpreter reads them: errors at any round, the call, the end.
fn vars() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let msg = pb.check_msg("result", Dir::To);
    let inc = pb.declare("inc", FuncKind::Function, None);
    let f = pb.declare("vars", FuncKind::Function, None);
    {
        let mut fb = pb.define(inc);
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let p = fb.addr(AddrSpace::Frame, s);
        let a = load(&mut fb, p, Width::B8);
        let (one, lo, hi) = (fb.int(1), fb.int(-1000), fb.int(1000));
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::BinRange {
            op: IntOp::Add,
            dst,
            a,
            b: one,
            lo,
            hi,
            what: msg,
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
    }
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    let (vx, vy, vz, vw) = (fb.slot(8, 8), fb.slot(4, 4), fb.slot(1, 1), fb.slot(8, 8));
    stmt(&mut fb);
    let (pa, pb_) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let a = load(&mut fb, pa, Width::B8);
    let b = load(&mut fb, pb_, Width::B8);
    let eight = fb.int(8);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Mod,
        dst: n,
        a: b,
        b: eight,
    });
    // x := 1; y := 2; z := 3; w := 4 (through an address a MakeDope uses)
    for (slot, w, v) in [(vx, Width::B8, 1), (vy, Width::B4, 2), (vz, Width::B1, 3)] {
        let at = fb.addr(AddrSpace::Frame, slot);
        let c = fb.int(v);
        fb.emit(Inst::Store {
            addr: at,
            src: c,
            w,
        });
    }
    let at = fb.addr(AddrSpace::Frame, vw);
    let four = fb.int(4);
    let parts = fb.regs(&[four]);
    fb.emit(Inst::MakeDope { at, parts });
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    let (i, one) = (fb.int(0), fb.int(1));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Lt,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    stmt(&mut fb);
    // x := x + a
    let (ax, x) = (fb.addr(AddrSpace::Frame, vx), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: x,
        addr: ax,
        w: Width::B8,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: x,
        a: x,
        b: a,
    });
    let ax2 = fb.addr(AddrSpace::Frame, vx);
    fb.emit(Inst::Store {
        addr: ax2,
        src: x,
        w: Width::B8,
    });
    // y := y * 3 + i, kept in 32 bits
    let (ay, y) = (fb.addr(AddrSpace::Frame, vy), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: y,
        addr: ay,
        w: Width::B4,
    });
    let three = fb.int(3);
    fb.emit(Inst::Bin {
        op: IntOp::Mul,
        dst: y,
        a: y,
        b: three,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: y,
        a: y,
        b: i,
    });
    let ay2 = fb.addr(AddrSpace::Frame, vy);
    fb.emit(Inst::Store {
        addr: ay2,
        src: y,
        w: Width::B4,
    });
    // z := z + x, kept in 8 bits; the callee reads nothing of ours
    let (az, z) = (fb.addr(AddrSpace::Frame, vz), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: z,
        addr: az,
        w: Width::B1,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: z,
        a: z,
        b: x,
    });
    let az2 = fb.addr(AddrSpace::Frame, vz);
    fb.emit(Inst::Store {
        addr: az2,
        src: z,
        w: Width::B1,
    });
    let args = fb.regs(&[i]);
    let k = fb.reg(RegTy::I64);
    fb.emit(Inst::Call {
        func: inc,
        args,
        dst: Some(k),
    });
    // w := w + k, written through the escaped address
    let aw = fb.addr(AddrSpace::Frame, vw);
    let w = load(&mut fb, aw, Width::B8);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: w,
        a: w,
        b: k,
    });
    let parts = fb.regs(&[w]);
    fb.emit(Inst::MakeDope { at: aw, parts });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    let mut sum = fb.int(0);
    for (slot, w) in [
        (vx, Width::B8),
        (vy, Width::B4),
        (vz, Width::B1),
        (vw, Width::B8),
    ] {
        let at = fb.addr(AddrSpace::Frame, slot);
        let v = load(&mut fb, at, w);
        let t = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: t,
            a: sum,
            b: v,
        });
        sum = t;
    }
    fb.emit(Inst::Ret { v: Some(sum) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn frame_variables_match_the_interpreter() {
    let (prog, f) = vars();
    let p = Both::new(prog);
    // Loop counts 0 to 7 (the last argument mod 8) with values that overflow at some round.
    for a in [0, 1, 7, 100, -3, 1 << 20, 1 << 61, i64::MAX, i64::MIN] {
        for b in [0, 1, 2, 3, 5, 7, 8, 9, -1] {
            let (x, y, n) = p.both(f, &[a as u64, b as u64]);
            assert_eq!(x, y, "a={a} b={b}");
            assert!(n > 0, "compiled code never ran");
        }
    }
    let (x, y, _) = p.both(f, &[3, 5]);
    assert!(x.is_ok() && x == y, "{x:?}");
}

/// `outer(n, c)` calls `inner(n, c)`, which builds on its native frame a string of `n` copies of
/// `c` (`NewFrame` with a fill), concatenates it with one more scalar element (`Cat`), fills a
/// word array (`Fill`) and returns the elements it can tell apart. Both functions run on native
/// frames and give the interpreter's results.
fn string_ops() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let inner = pb.declare("inner", FuncKind::Function, None);
    let outer = pb.declare("outer", FuncKind::Function, None);
    {
        let mut fb = pb.define(inner);
        let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        let sc = fb.param("c", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let (pn, pc) = (fb.addr(AddrSpace::Frame, sn), fb.addr(AddrSpace::Frame, sc));
        let (n, c) = (load(&mut fb, pn, Width::B8), load(&mut fb, pc, Width::B8));
        // The fill element: one byte.
        let eight = fb.int(8);
        let cell = alloc(&mut fb, eight);
        fb.emit(Inst::Store {
            addr: cell,
            src: c,
            w: Width::B1,
        });
        // s := n copies of c, kept by the frame.
        let s = fb.reg(RegTy::Ptr);
        fb.emit(Inst::NewFrame {
            dst: s,
            desc: cell,
            dsize: 0,
            data: cell,
            size: n,
            fill: 1,
        });
        stmt(&mut fb);
        // t := s & c (n + 1 bytes).
        let one = fb.int(1);
        let total = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: total,
            a: n,
            b: one,
        });
        let t = alloc(&mut fb, total);
        let parts = fb.regs(&[s, n, c, one]);
        fb.emit(Inst::Cat {
            dst: t,
            n: total,
            m: one,
            parts,
            scalars: 0b10,
            bias: 0,
            logic: false,
            w: Width::B1,
        });
        // w := three words of 0x01020304.
        let (twelve, k3, word) = (fb.int(12), fb.int(3), fb.int(0x0102_0304));
        let w = alloc(&mut fb, twelve);
        let e = alloc(&mut fb, eight);
        fb.emit(Inst::Store {
            addr: e,
            src: word,
            w: Width::B4,
        });
        fb.emit(Inst::Fill {
            dst: w,
            elem: e,
            n: k3,
            size: 4,
        });
        // The first element of s, the scalar piece of t, the last word of w.
        let (zero, eight_k, k8) = (fb.int(0), fb.int(8), fb.int(256));
        let first = index(&mut fb, s, zero);
        let a = load(&mut fb, first, Width::B1);
        let last = index(&mut fb, t, n);
        let b = load(&mut fb, last, Width::B1);
        let wl = index(&mut fb, w, eight_k);
        let d = load(&mut fb, wl, Width::B4);
        let (x, y) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
        fb.emit(Inst::Bin {
            op: IntOp::Mul,
            dst: x,
            a: b,
            b: k8,
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: y,
            a,
            b: x,
        });
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: r,
            a: y,
            b: d,
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(outer);
        let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        let sc = fb.param("c", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let (pn, pc) = (fb.addr(AddrSpace::Frame, sn), fb.addr(AddrSpace::Frame, sc));
        let (n, c) = (load(&mut fb, pn, Width::B8), load(&mut fb, pc, Width::B8));
        let args = fb.regs(&[n, c]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: inner,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    (pb.finish().unwrap(), outer)
}

/// `NewFrame`, `Cat` of non-logic elements and `Fill` run on native frames without a step
/// (docs/decisions.md "Compiled frames"): nothing is materialised, the results are the
/// interpreter's, and `outer` and `inner` both run on native frames.
#[test]
fn strings_on_native_frames() {
    let (prog, outer) = string_ops();
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 2);
    for (n, c) in [
        (0u64, 65u64),
        (1, 66),
        (7, 90),
        (64, 33),
        (300, 255),
        (5000, 1),
    ] {
        let (a, b, entries) = p.both(outer, &[n, c]);
        assert!(a.is_ok(), "{a:?}");
        assert_eq!(a, b, "n {n} c {c}");
        // The entry into `outer`, and at most the one materialisation when the temporary area
        // first has to grow.
        assert!(entries <= 3, "n {n} c {c}: {entries} entries");
    }
}

/// `inner(c)` keeps a four-element `std_ulogic_vector` of the code `c` in its own slots and calls
/// the native `is_x` on it with the array as values: the native reads the native frame's slots
/// through the data address (indirect addresses, which only the frame windows resolve). `outer(c)` calls `inner(c)`. `half(a)` is `ceil(a / 4)` through a
/// real native and `FToI`.
fn native_calls() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let is_x = pb.native(
        "yavs.ieee.std_logic_1164.is_x(std_ulogic_vector)boolean",
        None,
    );
    let ceil = pb.native("yavs.ieee.math_real.ceil(real)real", None);
    let inner = pb.declare("inner", FuncKind::Function, None);
    let outer = pb.declare("outer", FuncKind::Function, None);
    let half = pb.declare("half", FuncKind::Function, None);
    {
        let mut fb = pb.define(inner);
        let sc = fb.param("c", Width::B8, ParamClass::Value, Mode::In, None);
        let planes = fb.slot(32, 8);
        stmt(&mut fb);
        let pc = fb.addr(AddrSpace::Frame, sc);
        let c = load(&mut fb, pc, Width::B8);
        // plane p holds bit p of the code in each of the four elements (1111b or 0).
        let (two, fifteen) = (fb.int(2), fb.int(15));
        let pl = fb.addr(AddrSpace::Frame, planes);
        let mut q = c;
        for p in 0..4 {
            let bit = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Rem,
                dst: bit,
                a: q,
                b: two,
            });
            let m = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Mul,
                dst: m,
                a: bit,
                b: fifteen,
            });
            let at_p = fb.int(8 * p);
            let to = index(&mut fb, pl, at_p);
            fb.emit(Inst::Store {
                addr: to,
                src: m,
                w: Width::B8,
            });
            let next = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Div,
                dst: next,
                a: q,
                b: two,
            });
            q = next;
        }
        // The array as values: data, left 0, right 3, direction 0.
        let (l, r, d) = (fb.int(0), fb.int(3), fb.int(0));
        let args = fb.regs(&[pl, l, r, d]);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::CallNative {
            native: is_x,
            args,
            dst: Some(x),
        });
        fb.emit(Inst::Ret { v: Some(x) });
        fb.finish();
    }
    {
        let mut fb = pb.define(outer);
        let sc = fb.param("c", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let pc = fb.addr(AddrSpace::Frame, sc);
        let c = load(&mut fb, pc, Width::B8);
        let args = fb.regs(&[c]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: inner,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(half);
        let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let pa = fb.addr(AddrSpace::Frame, sa);
        let a = load(&mut fb, pa, Width::B8);
        let x = fb.reg(RegTy::F64);
        fb.emit(Inst::IToF { dst: x, src: a });
        let q = fb.reg(RegTy::F64);
        fb.emit(Inst::ConstF { dst: q, v: 0.25 });
        let y = fb.reg(RegTy::F64);
        fb.emit(Inst::FBin {
            op: FOp::Mul,
            dst: y,
            a: x,
            b: q,
        });
        let args = fb.regs(&[y]);
        let z = fb.reg(RegTy::F64);
        fb.emit(Inst::CallNative {
            native: ceil,
            args,
            dst: Some(z),
        });
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::FToI { dst: r, src: z });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    (pb.finish().unwrap(), outer, half)
}

/// A native call from a native frame runs without materialising it (the real arithmetic one),
/// also when it reads the frame's own slots through memory (the vector one).
#[test]
fn natives_called_from_native_frames() {
    let (prog, outer, half) = native_calls();
    let p = Both::new(prog);
    for c in 0..9u64 {
        let (a, b, entries) = p.both(outer, &[c]);
        assert!(a.is_ok(), "{a:?}");
        assert_eq!(a, b, "code {c}");
        assert!(entries <= 3, "code {c}: {entries} entries");
    }
    for a in [0u64, 1, 3, 4, 5, 100, 1 << 40, u64::MAX] {
        let (x, y, entries) = p.both(half, &[a]);
        assert_eq!(x, y, "half {a}");
        assert!(entries <= 3, "half {a}: {entries} entries");
    }
}

/// `top(n)` keeps `v := n`, calls its nested `bump(k)` (adds `k` to the parent's `v` through
/// `Up(1)` and calls `deeper`, nested in it, which reads both `v` through `Up(2)` and `bump`'s
/// own local through `Up(1)`), and returns `v`.
///
/// With `big` the nested procedure has a frame too large for a native frame, so it runs as
/// frame-mode code that reads the enclosing frames from the view.
fn nested(big: bool) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let top = pb.declare("top", FuncKind::Function, None);
    let bump = pb.declare("bump", FuncKind::Procedure, Some(top));
    let deeper = pb.declare("deeper", FuncKind::Function, Some(bump));
    // Slot offsets are decided as the functions are defined: top's `v` at 8 (after n).
    let (v_slot, b_local);
    {
        let mut fb = pb.define(top);
        let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        v_slot = fb.slot(8, 8);
        stmt(&mut fb);
        let pn = fb.addr(AddrSpace::Frame, sn);
        let n = load(&mut fb, pn, Width::B8);
        let pv = fb.addr(AddrSpace::Frame, v_slot);
        fb.emit(Inst::Store {
            addr: pv,
            src: n,
            w: Width::B8,
        });
        let three = fb.int(3);
        let args = fb.regs(&[three]);
        fb.emit(Inst::Call {
            func: bump,
            args,
            dst: None,
        });
        let args = fb.regs(&[n]);
        fb.emit(Inst::Call {
            func: bump,
            args,
            dst: None,
        });
        let r = load(&mut fb, pv, Width::B8);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    {
        let mut fb = pb.define(bump);
        let sk = fb.param("k", Width::B8, ParamClass::Value, Mode::In, None);
        b_local = fb.slot(8, 8);
        if big {
            fb.slot(8192, 8);
        }
        stmt(&mut fb);
        let pk = fb.addr(AddrSpace::Frame, sk);
        let k = load(&mut fb, pk, Width::B8);
        let pl = fb.addr(AddrSpace::Frame, b_local);
        fb.emit(Inst::Store {
            addr: pl,
            src: k,
            w: Width::B8,
        });
        let up = fb.addr(AddrSpace::Up(1), v_slot);
        let v = load(&mut fb, up, Width::B8);
        let sum = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: sum,
            a: v,
            b: k,
        });
        fb.emit(Inst::Store {
            addr: up,
            src: sum,
            w: Width::B8,
        });
        let args = fb.regs(&[k]);
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: deeper,
            args,
            dst: Some(d),
        });
        // v := v + deeper(k)
        let v = load(&mut fb, up, Width::B8);
        let sum = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: sum,
            a: v,
            b: d,
        });
        fb.emit(Inst::Store {
            addr: up,
            src: sum,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    {
        let mut fb = pb.define(deeper);
        let sk = fb.param("k", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let pk = fb.addr(AddrSpace::Frame, sk);
        let k = load(&mut fb, pk, Width::B8);
        let up1 = fb.addr(AddrSpace::Up(1), b_local);
        let l = load(&mut fb, up1, Width::B8);
        let up2 = fb.addr(AddrSpace::Up(2), v_slot);
        let v = load(&mut fb, up2, Width::B8);
        let t = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: t,
            a: l,
            b: v,
        });
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: r,
            a: t,
            b: k,
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    (pb.finish().unwrap(), top)
}

/// Nested subprograms run on native frames: their static links are passed to them and name
/// the frames of the enclosing subprograms (also when those are native frames themselves).
#[test]
fn nested_subprograms_on_native_frames() {
    let (prog, top) = nested(false);
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 3);
    for n in [0u64, 1, 7, 1000, 1 << 40] {
        let (a, b, entries) = p.both(top, &[n]);
        assert!(a.is_ok(), "{a:?}");
        assert_eq!(a, b, "n {n}");
        assert!(entries <= 3, "n {n}: {entries} entries");
    }
}

/// The same in frame-mode code: a variable of the enclosing subprogram one or two links up is
/// read and written in place from the view, whatever is stored there.
#[test]
fn enclosing_frames_in_frame_mode_code() {
    let (prog, top) = nested(true);
    let p = Both::new(prog);
    assert_eq!(p.tier.0.native_frames(), 1);
    for n in [0u64, 1, 7, 1000, 1 << 40] {
        let (a, b, _) = p.both(top, &[n]);
        assert!(a.is_ok(), "{a:?}");
        assert_eq!(a, b, "n {n}");
    }
}

/// A tier that leaves `interp` to the interpreter and counts the calls the interpreter makes
/// through `Tier::call` (native frames called from interpreted code, #510).
struct FromInterp {
    tier: LlvmTier,
    interp: Vec<FuncId>,
    calls: AtomicUsize,
}

impl Tier for FromInterp {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        if self.interp.contains(&func) {
            return None;
        }
        self.tier.run(func, pc, host)
    }

    fn ready(&self, func: FuncId) -> bool {
        !self.interp.contains(&func) && self.tier.ready(func)
    }

    fn callable(&self, func: FuncId) -> bool {
        self.tier.callable(func)
    }

    fn call(
        &self,
        func: FuncId,
        cells: &[u64],
        obj: Option<u64>,
        pc: u32,
        host: &mut dyn TierHost,
    ) -> Option<yavs_interp::Called> {
        let r = self.tier.call(func, cells, obj, pc, host);
        if r.is_some() {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
}

/// `f(args)` interpreted, then with `interp` interpreted and everything else compiled: both
/// results (value or error text) and the calls through `Tier::call`.
fn from_interp(
    prog: &Arc<Program>,
    interp: &[FuncId],
    f: FuncId,
    args: &[u64],
) -> (Result<u64, String>, Result<u64, String>, usize) {
    let tier = Arc::new(FromInterp {
        tier: yavs_llvm::compile(prog, OptLevel::O2, usize::MAX).unwrap(),
        interp: interp.to_vec(),
        calls: AtomicUsize::new(0),
    });
    let go = |t: Option<Arc<FromInterp>>| {
        let rt = Runtime::new(prog.clone(), Bindings::default()).unwrap();
        if let Some(t) = t {
            assert!(rt.set_tier(t));
        }
        yavs_interp::call(rt, f, Arc::new(InstData::default()), args)
            .map(|v| v.expect("a value"))
            .map_err(|e| e.to_string())
    };
    let (a, b) = (go(None), go(Some(tier.clone())));
    (a, b, tier.calls.load(Ordering::Relaxed))
}

/// The interpreter calls compiled functions on native frames (`Tier::call`): the results, an
/// error inside the callee (its native frame left to the interpreter, which reports it), a
/// recursion past the native stack allowance and past the depth limit are the interpreter's.
#[test]
fn interpreted_callers_call_native_frames() {
    let (prog, run, depth) = calls();
    let prog = Arc::new(prog);
    let (a, b, n) = from_interp(&prog, &[run, depth], run, &[100_000]);
    assert_eq!((a, b), (Ok(100_000), Ok(100_000)));
    assert_eq!(n, 100_000, "one native call per call of `inc`");
    let (a, b, _) = from_interp(&prog, &[run, depth], run, &[2_000_000]);
    assert!(a.as_ref().is_err_and(|e| e.contains("result")), "{a:?}");
    assert_eq!(a, b);
    for k in [5000, 20_000] {
        let (a, b, n) = from_interp(&prog, &[run, depth], depth, &[k]);
        assert_eq!(a, b, "depth {k}");
        assert!(n >= 1, "depth {k}: no native call");
    }
}

/// `held(k)`: a new object of eight 8-byte elements, `Deref`ed once, element `k` written with
/// `k + 100` and read back through two `Index`es of the same address (the object stays in hand
/// between the instructions); the sum of the element read and element 0 is returned. `k` out of
/// range, or past 4 GiB, is the interpreter's error at the same instruction.
fn held() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int {
        lo: i64::MIN,
        hi: i64::MAX,
    });
    let what = pb.check_msg("p.all", Dir::To);
    let f = pb.declare("held", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let s = fb.param("k", Width::B8, ParamClass::Value, Mode::In, None);
    stmt(&mut fb);
    let pk = fb.addr(AddrSpace::Frame, s);
    let k = load(&mut fb, pk, Width::B8);
    let size = fb.int(64);
    let init = alloc(&mut fb, size);
    let h = fb.reg(RegTy::Handle);
    fb.emit(Inst::New {
        dst: h,
        ty: int,
        init,
        size,
        shared: false,
    });
    let p = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Deref {
        dst: p,
        handle: h,
        what,
    });
    let (zero, hundred) = (fb.int(0), fb.int(100));
    let at0 = index(&mut fb, p, zero);
    fb.emit(Inst::Store {
        addr: at0,
        src: hundred,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let atk = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: atk,
        base: p,
        pos: k,
        stride: 8,
    });
    let v = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: v,
        a: k,
        b: eight,
    });
    fb.emit(Inst::Store {
        addr: atk,
        src: v,
        w: Width::B8,
    });
    // The rest in a block of its own that only this one jumps to: the object is still in hand.
    let next = fb.block();
    fb.emit(Inst::Br { to: next });
    fb.at(next);
    let back = load(&mut fb, atk, Width::B8);
    let first = load(&mut fb, at0, Width::B8);
    let r = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: r,
        a: back,
        b: first,
    });
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// A heap object a `Deref` found stays in hand for the instructions after it in a native frame
/// (no second lookup); addresses derived from it keep every check of the interpreter.
#[test]
fn heap_object_held_between_instructions() {
    let (prog, f) = held();
    let p = Both::new(prog);
    for k in [0u64, 1, 7, 8, 100, 1 << 32, (1 << 32) + 1, u64::MAX] {
        let (a, b, _) = p.both(f, &[k]);
        assert_eq!(a, b, "{k}");
        assert_eq!(a.is_ok(), k < 8, "{k}: {a:?}");
    }
}

/// `up(l, r, dir, k, x, start, e)` and `down(..)`: a descriptor `l, r, dir` over a 16-byte array of
/// zeros but for a 7 at element `k`, and the search loop the interpreter runs as one op (`Pos`,
/// `Index`, `Load`, compare with `x`, compare of the index with `e`, step by 1 or -1) from `start`:
/// `i * 1000 + element` where the element matches, `-i` at `e`.
fn search_loop(step: IntOp) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let what = pb.check_msg("index", Dir::To);
    let f = pb.declare("search", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let names = ["l", "r", "dir", "k", "x", "start", "e"];
    let slots: Vec<u32> = names
        .iter()
        .map(|n| fb.param(n, Width::B8, ParamClass::Value, Mode::In, None))
        .collect();
    stmt(&mut fb);
    let v: Vec<Reg> = slots
        .iter()
        .map(|&s| {
            let p = fb.addr(AddrSpace::Frame, s);
            load(&mut fb, p, Width::B8)
        })
        .collect();
    let (l, r, dir, k, x, start, e) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
    let d = dope(&mut fb, l, r, dir);
    let sixteen = fb.int(16);
    let arr = alloc(&mut fb, sixteen);
    let seven = fb.int(7);
    let at = index(&mut fb, arr, k);
    fb.emit(Inst::Store {
        addr: at,
        src: seven,
        w: Width::B1,
    });
    let (head, latch, stepb, found, end) =
        (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    let (i, one) = (fb.reg(RegTy::I64), fb.int(1));
    fb.emit(Inst::Move { dst: i, src: start });
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let (p, a, ch, c) = (
        fb.reg(RegTy::I64),
        fb.reg(RegTy::Ptr),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Pos {
        dst: p,
        dope: d,
        dim: 0,
        idx: i,
        what,
    });
    fb.emit(Inst::Index {
        dst: a,
        base: arr,
        pos: p,
        stride: 1,
    });
    fb.emit(Inst::Load {
        dst: ch,
        addr: a,
        w: Width::B1,
    });
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: ch,
        b: x,
    });
    fb.emit(Inst::CondBr {
        c,
        t: found,
        f: latch,
    });
    fb.at(latch);
    let c2 = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c2,
        a: i,
        b: e,
    });
    fb.emit(Inst::CondBr {
        c: c2,
        t: end,
        f: stepb,
    });
    fb.at(stepb);
    fb.emit(Inst::Bin {
        op: step,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(found);
    let (k1000, m, r1) = (fb.int(1000), fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Bin {
        op: IntOp::Mul,
        dst: m,
        a: i,
        b: k1000,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: r1,
        a: m,
        b: ch,
    });
    fb.emit(Inst::Ret { v: Some(r1) });
    fb.at(end);
    let (zero, r2) = (fb.int(0), fb.reg(RegTy::I64));
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: r2,
        a: zero,
        b: i,
    });
    fb.emit(Inst::Ret { v: Some(r2) });
    fb.finish();
    // `outer` calls it, so it runs on a native frame.
    let outer = pb.declare("outer", FuncKind::Function, None);
    let mut fb = pb.define(outer);
    let slots: Vec<u32> = names
        .iter()
        .map(|n| fb.param(n, Width::B8, ParamClass::Value, Mode::In, None))
        .collect();
    stmt(&mut fb);
    let v: Vec<Reg> = slots
        .iter()
        .map(|&s| {
            let p = fb.addr(AddrSpace::Frame, s);
            load(&mut fb, p, Width::B8)
        })
        .collect();
    // Room in the temporary area, which native frames do not grow.
    let room = fb.int(4096);
    let spare = alloc(&mut fb, room);
    let zero = fb.int(0);
    fb.emit(Inst::Store {
        addr: spare,
        src: zero,
        w: Width::B1,
    });
    stmt(&mut fb);
    let args = fb.regs(&v);
    let r = fb.reg(RegTy::I64);
    fb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(r),
    });
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    (pb.finish().unwrap(), outer)
}

/// A search loop in a native frame runs through the scan helper and gives the interpreter's
/// result: found, ended, an index outside the descriptor, bytes outside the array, either
/// direction, either step.
#[test]
fn search_loops_match_the_interpreter() {
    let scans = || {
        yavs_llvm::callback_counts()
            .and_then(|c| c.into_iter().find(|c| c.0 == "scan").map(|c| c.1))
            .unwrap_or(0)
    };
    let before = scans();
    for (step, dirs) in [(IntOp::Add, [0u64, 1]), (IntOp::Sub, [1, 0])] {
        let (prog, f) = search_loop(step);
        let p = Both::new(prog);
        assert_eq!(p.tier.0.native_frames(), 2);
        // The loop runs along the direction of the descriptor for the step that goes with it.
        let dir = dirs[0];
        for (l, r) in [
            (0i64, 15i64),
            (1, 16),
            (3, 12),
            (0, 20),
            (15, 0),
            (16, 1),
            (20, 0),
        ] {
            for k in [0u64, 5, 15, 16] {
                for x in [0u64, 7, 9] {
                    for (start, e) in [(l, r), (l, l), (r, l), (l + 1, r), (r + 2, l), (l - 1, r)] {
                        let args = [l as u64, r as u64, dir, k, x, start as u64, e as u64];
                        let (a, b, _) = p.both(f, &args);
                        assert_eq!(a, b, "{step:?} {args:?}");
                    }
                }
            }
        }
    }
    if cfg!(debug_assertions) {
        assert!(scans() > before, "no loop went through the helper");
    }
}

/// `heap(n)`: a new integer object incremented through its handle `n` times (a `Deref` each
/// time, found again in the object table after the first), its value returned; for `n` over 100
/// the object is deallocated first, so the last `Deref` is the interpreter's stale-handle error.
/// `outer(n) = heap(n)`.
fn heap_calls() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int {
        lo: i64::MIN,
        hi: i64::MAX,
    });
    let what = pb.check_msg("p.all", Dir::To);
    let heap = pb.declare("heap", FuncKind::Function, None);
    let outer = pb.declare("outer", FuncKind::Function, None);
    {
        let mut fb = pb.define(heap);
        let s = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        let var = fb.local("v", int, 8, 8);
        stmt(&mut fb);
        let pn = fb.addr(AddrSpace::Frame, s);
        let n = load(&mut fb, pn, Width::B8);
        let eight = fb.int(8);
        let init = alloc(&mut fb, eight);
        let zero = fb.int(0);
        fb.emit(Inst::Store {
            addr: init,
            src: zero,
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
        let (head, body, done, free, out) =
            (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
        let (i, one) = (fb.int(0), fb.int(1));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
            dst: c,
            a: i,
            b: n,
        });
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: done,
        });
        fb.at(body);
        stmt(&mut fb);
        let p = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        let v = load(&mut fb, p, Width::B8);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: v,
            a: v,
            b: one,
        });
        fb.emit(Inst::Store {
            addr: p,
            src: v,
            w: Width::B8,
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(done);
        let (hundred, big) = (fb.int(100), fb.reg(RegTy::I64));
        fb.emit(Inst::Cmp {
            op: CmpOp::Gt,
            dst: big,
            a: n,
            b: hundred,
        });
        fb.emit(Inst::CondBr {
            c: big,
            t: free,
            f: out,
        });
        fb.at(free);
        // `deallocate` of a variable holding a copy of the handle: `h` is stale then.
        let a = fb.addr(AddrSpace::Frame, var);
        fb.emit(Inst::Store {
            addr: a,
            src: h,
            w: Width::B8,
        });
        fb.emit(Inst::Free { ptr: a });
        fb.emit(Inst::Br { to: out });
        fb.at(out);
        let p = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        let v = load(&mut fb, p, Width::B8);
        fb.emit(Inst::Ret { v: Some(v) });
        fb.finish();
    }
    {
        let mut fb = pb.define(outer);
        let s = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let pn = fb.addr(AddrSpace::Frame, s);
        let n = load(&mut fb, pn, Width::B8);
        let args = fb.regs(&[n]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: heap,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    (pb.finish().unwrap(), heap, outer)
}

/// Heap objects a `Deref` found stay in the object table across instructions the interpreter
/// executes (`Mem::forget_since`); a deallocation empties it, so a stale handle is the
/// interpreter's error, in frame-mode code and on a native frame called from the interpreter.
#[test]
fn heap_objects_found_again_until_deallocated() {
    let (prog, heap, outer) = heap_calls();
    let prog = Arc::new(prog);
    for n in [0u64, 1, 50, 200] {
        let (a, b, k) = from_interp(&prog, &[outer], outer, &[n]);
        assert_eq!(a, b, "native frame, n {n}");
        assert_eq!(k, 1, "n {n}");
        assert_eq!(a.is_ok(), n <= 100, "{a:?}");
        let (a, b, _) = from_interp(&prog, &[], heap, &[n]);
        assert_eq!(a, b, "frame mode, n {n}");
    }
}

/// `f(n, c)` entered from the interpreter, so in frame mode: a native computing from its
/// arguments (`ceil`) and one reading the frame's slots through memory (`is_x`), a `Cat` of
/// non-logic elements, and the layout of a one-dimensional array of `n` elements; the result
/// mixes them all.
fn frame_mode_ops() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let ceil = pb.native("yavs.ieee.math_real.ceil(real)real", None);
    let is_x = pb.native(
        "yavs.ieee.std_logic_1164.is_x(std_ulogic_vector)boolean",
        None,
    );
    let byte = pb.ty(Ty::Int { lo: 0, hi: 255 });
    let arr = pb.ty(Ty::Array {
        elem: byte,
        elem_desc: None,
        dims: 1,
    });
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let sc = fb.param("c", Width::B8, ParamClass::Value, Mode::In, None);
    let planes = fb.slot(32, 8);
    stmt(&mut fb);
    let (pn, pc) = (fb.addr(AddrSpace::Frame, sn), fb.addr(AddrSpace::Frame, sc));
    let (n, c) = (load(&mut fb, pn, Width::B8), load(&mut fb, pc, Width::B8));
    // ceil(n / 4)
    let x = fb.reg(RegTy::F64);
    fb.emit(Inst::IToF { dst: x, src: n });
    let q = fb.reg(RegTy::F64);
    fb.emit(Inst::ConstF { dst: q, v: 0.25 });
    let y = fb.reg(RegTy::F64);
    fb.emit(Inst::FBin {
        op: FOp::Mul,
        dst: y,
        a: x,
        b: q,
    });
    let args = fb.regs(&[y]);
    let z = fb.reg(RegTy::F64);
    fb.emit(Inst::CallNative {
        native: ceil,
        args,
        dst: Some(z),
    });
    let halves = fb.reg(RegTy::I64);
    fb.emit(Inst::FToI {
        dst: halves,
        src: z,
    });
    // is_x on four elements whose planes are the bits of the low nibble of c, in the frame
    let pl = fb.addr(AddrSpace::Frame, planes);
    let nib = fb.int(15);
    let low = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Rem,
        dst: low,
        a: c,
        b: nib,
    });
    for p in 0..4 {
        let at_p = fb.int(8 * p);
        let to = index(&mut fb, pl, at_p);
        fb.emit(Inst::Store {
            addr: to,
            src: low,
            w: Width::B8,
        });
    }
    let (l, r, d) = (fb.int(0), fb.int(3), fb.int(0));
    let args = fb.regs(&[pl, l, r, d]);
    let isx = fb.reg(RegTy::I64);
    fb.emit(Inst::CallNative {
        native: is_x,
        args,
        dst: Some(isx),
    });
    // The layout of an array of n elements: [size, align, stride, count].
    let (zero, one, dir) = (fb.int(0), fb.int(1), fb.int(0));
    let last = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: last,
        a: n,
        b: one,
    });
    let n24 = fb.int(24);
    let desc = alloc(&mut fb, n24);
    let parts = fb.regs(&[zero, last, dir]);
    fb.emit(Inst::MakeDope { at: desc, parts });
    let lay = fb.reg(RegTy::Ptr);
    fb.emit(Inst::LayoutOf {
        dst: lay,
        ty: arr,
        desc,
        cache: None,
    });
    let size = load(&mut fb, lay, Width::B8);
    let sixteen_k = fb.int(16);
    let sp = index(&mut fb, lay, sixteen_k);
    let stride = load(&mut fb, sp, Width::B8);
    // s := the byte at `cell`, then c.
    let total = fb.int(2);
    let eight = fb.int(8);
    let cell = alloc(&mut fb, eight);
    fb.emit(Inst::Store {
        addr: cell,
        src: c,
        w: Width::B1,
    });
    let s = alloc(&mut fb, eight);
    let parts = fb.regs(&[cell, one, c, one]);
    fb.emit(Inst::Cat {
        dst: s,
        n: total,
        m: one,
        parts,
        scalars: 0b10,
        bias: 0,
        logic: false,
        w: Width::B1,
    });
    let at = index(&mut fb, s, one);
    let tail = load(&mut fb, at, Width::B1);
    // Mix.
    let mut acc = halves;
    for (v, k) in [(isx, 3), (size, 7), (stride, 11), (tail, 13)] {
        let kk = fb.int(k);
        let m = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Mul,
            dst: m,
            a: acc,
            b: kk,
        });
        let sum = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: sum,
            a: m,
            b: v,
        });
        acc = sum;
    }
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// Natives, a `Cat` and a one-dimensional `LayoutOf` in frame-mode code give the interpreter's
/// results and run compiled, one entry into compiled code.
#[test]
fn natives_strings_and_layouts_in_frame_mode() {
    let (prog, f) = frame_mode_ops();
    let p = Both::new(prog);
    for (n, c) in [
        (1u64, 3u64),
        (0, 5),
        (7, 15),
        (64, 33),
        (300, 255),
        (5000, 1),
        (1 << 20, 9),
    ] {
        let (a, b, entries) = p.both(f, &[n, c]);
        assert_eq!(a, b, "n {n} c {c}");
        assert!(a.is_ok() || n == 0, "{a:?}");
        assert_eq!(entries, 1, "n {n} c {c}");
    }
}

/// A loop that redefines a register with a cached `LayoutOf` right after a deferred definition
/// of it (#710): the layout the interpreter made must be what the frame keeps, not the value
/// the earlier definition deferred.
fn redefined_by_cached_layout() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let byte = pb.ty(Ty::Int { lo: 0, hi: 255 });
    let arr = pb.ty(Ty::Array {
        elem: byte,
        elem_desc: None,
        dims: 1,
    });
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    // The kept layout: a flag word, the 24-byte descriptor, the four layout words.
    let kept = fb.slot(64, 8);
    let sz = fb.param("z", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    stmt(&mut fb);
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = load(&mut fb, pn, Width::B8);
    let pz = fb.addr(AddrSpace::Frame, sz);
    let z = load(&mut fb, pz, Width::B8);
    let (zero, one, dir, sixty4) = (fb.int(0), fb.int(1), fb.int(0), fb.int(64));
    // Four words the layout is not, for the register to point at.
    let junk = alloc(&mut fb, sixty4);
    let junk_word = fb.int(7777);
    for k in 0..4 {
        let at = fb.int(8 * k);
        let to = index(&mut fb, junk, at);
        fb.emit(Inst::Store {
            addr: to,
            src: junk_word,
            w: Width::B8,
        });
    }
    let eight = fb.int(8);
    let junk8 = index(&mut fb, junk, eight);
    let last = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: last,
        a: n,
        b: one,
    });
    let desc = dope(&mut fb, zero, last, dir);
    let (i, acc, three) = (fb.int(0), fb.int(0), fb.int(2));
    let lay = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Le,
        dst: c,
        a: i,
        b: three,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    fb.emit(Inst::Select {
        dst: lay,
        c: z,
        a: junk8,
        b: junk,
    });
    // The register is live here, so its definition stays.
    let jv = load(&mut fb, lay, Width::B8);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: acc,
        a: acc,
        b: jv,
    });
    fb.emit(Inst::LayoutOf {
        dst: lay,
        ty: arr,
        desc,
        cache: Some(kept),
    });
    let size = load(&mut fb, lay, Width::B8);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: acc,
        a: acc,
        b: size,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn cached_layout_replaces_a_deferred_value_of_its_register() {
    let (prog, f) = redefined_by_cached_layout();
    let p = Both::new(prog);
    for n in [1u64, 5, 100] {
        let (a, b, entries) = p.both(f, &[n, 0]);
        assert_ne!(a, Ok(6 * 7777), "n {n}");
        assert_eq!(a, b, "n {n}");
        assert_eq!(entries, 1, "n {n}");
    }
}

/// The compile thread keeps the CPU mask the process was started with (#681): it is not moved to
/// the CPUs outside it, which on a shared host belong to other pinned runs.
#[cfg(target_os = "linux")]
#[test]
#[allow(unsafe_code)] // sched_setaffinity, to restrict this thread
fn compile_thread_keeps_the_affinity_of_the_process() {
    unsafe extern "C" {
        fn sched_getaffinity(pid: i32, size: usize, mask: *mut u64) -> i32;
        fn sched_setaffinity(pid: i32, size: usize, mask: *const u64) -> i32;
    }
    // (`None` for a task that ended meanwhile: other tests' compile threads come and go.)
    let allowed = |tid: &str| -> Option<String> {
        let s = std::fs::read_to_string(format!("/proc/self/task/{tid}/status")).ok()?;
        s.lines()
            .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
            .map(|l| l.trim().to_string())
    };
    // Restrict this test thread (threads it starts inherit the mask) to one CPU it may use.
    let (mut old, mut one) = ([0u64; 16], [0u64; 16]);
    let size = std::mem::size_of_val(&old);
    // SAFETY: pid 0 is the calling thread; both masks hold `size` bytes.
    unsafe {
        assert_eq!(sched_getaffinity(0, size, old.as_mut_ptr()), 0);
        let cpu = (0..64 * 16)
            .find(|c| old[c / 64] >> (c % 64) & 1 == 1)
            .unwrap();
        one[cpu / 64] = 1 << (cpu % 64);
        assert_eq!(sched_setaffinity(0, size, one.as_ptr()), 0);
    }
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let note: Note = Box::new(|_, _, _| {});
    let tier = AutoTier::start(&rt, OptLevel::O2, usize::MAX, HOT, note, None).unwrap();
    // The compile thread works only on a hot function: make one, then look at its mask.
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]).unwrap();
    }
    let mine = allowed(
        &std::fs::read_link("/proc/thread-self")
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
    )
    .unwrap();
    let mut found = 0;
    for e in std::fs::read_dir("/proc/self/task").unwrap() {
        let Ok(e) = e else { continue };
        let tid = e.file_name().to_string_lossy().into_owned();
        let Ok(comm) = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm")) else {
            continue;
        };
        if comm.trim() == "yavs-tier-up" {
            // (Compile threads of tests running at the same time have the full mask.)
            found += usize::from(allowed(&tid).as_deref() == Some(mine.as_str()));
        }
    }
    assert!(found >= 1, "no compile thread with the mask {mine}");
    tier.stop(true);
    // SAFETY: as above.
    unsafe {
        sched_setaffinity(0, size, old.as_ptr());
    }
}
