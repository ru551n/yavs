//! JIT spike tests. Run with `cargo test -p yavs-llvm --features llvm`.
#![cfg(feature = "llvm")]
#![allow(unsafe_code)] // calling JIT code is inherently unsafe

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use yavs_llvm::spike::{
    self, CHAIN_FIELDS, ChainFn, OVERFLOW_CALLBACK, ProcessFn, SigState, SumFn,
};
use yavs_llvm::{Jit, Module, OptLevel};

thread_local! {
    /// Statement id reported by the last overflow callback on this thread.
    static OVERFLOW: Cell<Option<i64>> = const { Cell::new(None) };
}

/// The Rust runtime function the JIT code calls on overflow.
extern "C" fn yavs_rt_overflow(stmt: i64) {
    OVERFLOW.with(|o| o.set(Some(stmt)));
}

fn take_overflow() -> Option<i64> {
    OVERFLOW.with(|o| o.take())
}

fn runtime_symbols() -> [(&'static str, usize); 1] {
    [(OVERFLOW_CALLBACK, yavs_rt_overflow as *const () as usize)]
}

fn jit_with(build: impl FnOnce(&Module), opt: OptLevel) -> Jit {
    let mut m = Module::new("spike");
    build(&m);
    m.verify().expect("generated IR verifies");
    m.optimize(opt).expect("passes run");
    m.verify().expect("optimised IR verifies");
    let mut jit = Jit::new(opt, &runtime_symbols()).expect("LLJIT created");
    jit.add(m).expect("module added");
    jit
}

fn call_sum(f: SumFn, data: &[i64]) -> (i32, i64) {
    let mut out = 0;
    // SAFETY: f is checked_sum; data and out are valid for the call.
    let rc = unsafe { f(data.as_ptr(), data.len() as i64, &mut out) };
    (rc, out)
}

#[test]
fn checked_sum_and_overflow_callback() {
    for opt in [OptLevel::O0, OptLevel::O2] {
        let mut jit = jit_with(spike::add_checked_sum, opt);
        let addr = jit.lookup("checked_sum").unwrap();
        // SAFETY: the symbol has the SumFn signature.
        let f: SumFn = unsafe { std::mem::transmute(addr) };

        let data: Vec<i64> = (1..=1000).collect();
        assert_eq!(call_sum(f, &data), (0, 500_500));
        assert_eq!(take_overflow(), None);
        assert_eq!(call_sum(f, &[]), (0, 0));

        assert_eq!(call_sum(f, &[1, i64::MAX - 1, 1, 5]).0, 1);
        assert_eq!(take_overflow(), Some(2), "overflow reports element index");
        assert_eq!(call_sum(f, &[i64::MIN, -1]).0, 1);
        assert_eq!(take_overflow(), Some(1));
    }
}

#[test]
fn process_body_reads_and_writes_struct() {
    let mut jit = jit_with(spike::add_process_body, OptLevel::O2);
    // SAFETY: the symbol has the ProcessFn signature.
    let f: ProcessFn = unsafe { std::mem::transmute(jit.lookup("process_body").unwrap()) };

    let mut s = SigState {
        cur: 20,
        next: 0,
        event: 0,
        runs: 3,
    };
    // SAFETY: s is a valid SigState.
    assert_eq!(unsafe { f(&mut s) }, 0);
    assert_eq!(
        s,
        SigState {
            cur: 20,
            next: 42,
            event: 1,
            runs: 4
        }
    );

    let mut s = SigState {
        cur: -2,
        ..SigState::default()
    };
    // SAFETY: as above.
    assert_eq!(unsafe { f(&mut s) }, 0);
    assert_eq!((s.next, s.event), (-2, 0), "no event when value unchanged");

    let mut s = SigState {
        cur: i64::MAX / 2 + 1,
        ..SigState::default()
    };
    // SAFETY: as above.
    assert_eq!(unsafe { f(&mut s) }, 1);
    assert_eq!(take_overflow(), Some(1), "multiply overflow is statement 1");
    assert_eq!(s.runs, 0, "nothing written after the trap");

    let mut s = SigState {
        cur: i64::MAX / 2,
        ..SigState::default()
    };
    // SAFETY: as above.
    assert_eq!(unsafe { f(&mut s) }, 1);
    assert_eq!(take_overflow(), Some(2), "add overflow is statement 2");
}

#[test]
fn unresolved_runtime_symbol_is_a_clear_error() {
    let mut m = Module::new("spike");
    spike::add_checked_sum(&m);
    m.optimize(OptLevel::O2).unwrap();
    let mut jit = Jit::new(OptLevel::O2, &[]).unwrap(); // callback not provided
    jit.add(m).unwrap();
    let err = jit.lookup("checked_sum").unwrap_err();
    assert!(
        err.0.contains(OVERFLOW_CALLBACK),
        "error names the symbol: {err}"
    );
    let err = jit.lookup("no_such_function").unwrap_err();
    assert!(err.0.contains("no_such_function"), "{err}");
}

#[test]
fn jit_targets_portable_cpu_on_host_triple() {
    let jit = Jit::new(OptLevel::O2, &[]).unwrap();
    let triple = jit.triple();
    let arch = std::env::consts::ARCH;
    assert!(triple.starts_with(arch), "{triple} vs {arch}");
}

#[test]
fn chain_matches_reference_semantics() {
    const STMTS: usize = 800; // about 10k IR instructions
    let mut m = Module::new("chain");
    spike::add_chain(&m, STMTS);
    assert!(m.instruction_count() > 10_000, "{}", m.instruction_count());
    m.verify().unwrap();
    m.optimize(OptLevel::O2).unwrap();
    let mut jit = Jit::new(OptLevel::O2, &runtime_symbols()).unwrap();
    jit.add(m).unwrap();
    // SAFETY: the symbol has the ChainFn signature.
    let f: ChainFn = unsafe { std::mem::transmute(jit.lookup("chain").unwrap()) };

    let init: [i64; CHAIN_FIELDS] = std::array::from_fn(|i| (i as i64) * 1_000_003 - 7);
    let (mut got, mut want) = (init, init);
    // SAFETY: got has CHAIN_FIELDS elements.
    assert_eq!(unsafe { f(got.as_mut_ptr()) }, 0);
    assert_eq!(spike::chain_reference(&mut want, STMTS), Ok(()));
    assert_eq!(got, want);

    let mut init = init;
    init[0] = i64::MAX - 100; // statement 0 reads field 0
    let (mut got, mut want) = (init, init);
    // SAFETY: as above.
    let rc = unsafe { f(got.as_mut_ptr()) };
    let reference = spike::chain_reference(&mut want, STMTS);
    assert_eq!((rc, take_overflow()), (1, reference.err()));
    assert_eq!(got, want, "writes before the trap match the reference");
}

/// A module of more than 100 000 LLVM instructions takes the pipeline without `simplifycfg` and
/// `adce` (quadratic in the module, #630); the code is the same function.
#[test]
fn a_huge_module_matches_reference_semantics() {
    const STMTS: usize = 9_000; // about 120k LLVM instructions
    let mut m = Module::new("chain");
    spike::add_chain(&m, STMTS);
    assert!(m.instruction_count() > 100_000, "{}", m.instruction_count());
    m.verify().unwrap();
    m.optimize(OptLevel::Fast).unwrap();
    let mut jit = Jit::new(OptLevel::Fast, &runtime_symbols()).unwrap();
    jit.add(m).unwrap();
    // SAFETY: the symbol has the ChainFn signature.
    let f: ChainFn = unsafe { std::mem::transmute(jit.lookup("chain").unwrap()) };
    let init: [i64; CHAIN_FIELDS] = std::array::from_fn(|i| (i as i64) * 1_000_003 - 7);
    let (mut got, mut want) = (init, init);
    // SAFETY: got has CHAIN_FIELDS elements.
    assert_eq!(unsafe { f(got.as_mut_ptr()) }, 0);
    assert_eq!(spike::chain_reference(&mut want, STMTS), Ok(()));
    assert_eq!(got, want);
}

/// The interpreter stand-in for `checked_sum`.
unsafe extern "C" fn interp_sum(data: *const i64, n: i64, out: *mut i64) -> i32 {
    // SAFETY: same contract as the JIT version.
    let data = unsafe { std::slice::from_raw_parts(data, n as usize) };
    let mut acc = 0i64;
    for (i, &v) in data.iter().enumerate() {
        match acc.checked_add(v) {
            Some(s) => acc = s,
            None => {
                yavs_rt_overflow(i as i64);
                return 1;
            }
        }
    }
    // SAFETY: out is valid.
    unsafe { *out = acc };
    0
}

/// The tiering seam: the simulation thread calls through an atomic function
/// pointer that starts at the interpreter. One background thread compiles and
/// swaps in the JIT code without the simulation thread ever waiting.
#[test]
fn background_compile_swaps_function_pointer() {
    let slot = Arc::new(AtomicUsize::new(interp_sum as *const () as usize));
    let compiling = Arc::new(AtomicBool::new(true));

    let compiler = {
        let (slot, compiling) = (slot.clone(), compiling.clone());
        std::thread::spawn(move || {
            let mut jit = jit_with(spike::add_checked_sum, OptLevel::O2);
            let addr = jit.lookup("checked_sum").unwrap();
            slot.store(addr, Ordering::Release);
            compiling.store(false, Ordering::Release);
            jit // keeps the code alive; the caller owns it from here
        })
    };

    let data: Vec<i64> = (0..64).collect();
    let (mut interp_calls, mut calls_during_compile) = (0u64, 0u64);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let during = compiling.load(Ordering::Acquire);
        let addr = slot.load(Ordering::Acquire);
        // SAFETY: the slot only ever holds a SumFn.
        let f: SumFn = unsafe { std::mem::transmute(addr) };
        assert_eq!(call_sum(f, &data), (0, 2016));
        if addr == interp_sum as *const () as usize {
            interp_calls += 1;
            calls_during_compile += u64::from(during);
        } else {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background compile never finished"
        );
    }
    let jit = compiler.join().unwrap();

    // The JIT tier keeps the interpreter's semantics, including the trap.
    // SAFETY: the slot holds the JIT checked_sum, alive while `jit` is.
    let f: SumFn = unsafe { std::mem::transmute(slot.load(Ordering::Acquire)) };
    assert_eq!(call_sum(f, &[i64::MAX, 1]).0, 1);
    assert_eq!(take_overflow(), Some(1));
    assert_eq!(call_sum(interp_sum, &[i64::MAX, 1]).0, 1);
    assert_eq!(take_overflow(), Some(1));

    assert!(
        calls_during_compile > 0,
        "simulation thread kept running while compiling"
    );
    eprintln!("interpreter calls before swap: {interp_calls}");
    drop(jit);
}

/// The unoptimised code generator reads the low half of a load as a 4-byte load: no 10-byte
/// `movabs` of the mask (its immediate would be `ff ff ff ff 00 00 00 00` in the object).
#[test]
fn low_half_of_a_load_is_a_narrow_load() {
    for opt in [OptLevel::O0, OptLevel::Fast, OptLevel::FastRa] {
        let mut m = Module::new("lowhalf");
        spike::add_low_half(&m);
        m.optimize(opt).expect("passes run");
        let obj = m.emit(opt).expect("object");
        let mask = [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0];
        assert!(
            !obj.windows(8).any(|w| w == mask),
            "{opt:?}: the mask is still an immediate"
        );
        let mut jit = Jit::new(opt, &runtime_symbols()).expect("LLJIT created");
        jit.add_object(&obj).expect("object added");
        let addr = jit.lookup("low_half").unwrap();
        // SAFETY: the symbol is `extern "C" fn(*const i64) -> i64`.
        let f: unsafe extern "C" fn(*const i64) -> i64 = unsafe { std::mem::transmute(addr) };
        let x: i64 = 0x1122_3344_8899_aabbu64 as i64;
        // SAFETY: x is valid for the call.
        assert_eq!(unsafe { f(&x) }, 0x8899_aabb);
    }
}

/// `FastRa` (the optimised register allocator on the unoptimised selector, per target machine, #715)
/// makes other code than `Fast`, smaller where values live across many blocks, that gives the
/// reference's results.
#[test]
fn fast_ra_makes_smaller_correct_code() {
    const STMTS: usize = 600;
    let mut sizes = Vec::new();
    for opt in [OptLevel::Fast, OptLevel::FastRa] {
        let mut m = Module::new("chain");
        spike::add_chain(&m, STMTS);
        m.verify().unwrap();
        m.optimize(opt).unwrap();
        let obj = m.emit(opt).unwrap();
        sizes.push(obj.len());
        let mut jit = Jit::new(opt, &runtime_symbols()).unwrap();
        jit.add_object(&obj).unwrap();
        // SAFETY: the symbol has the ChainFn signature.
        let f: ChainFn = unsafe { std::mem::transmute(jit.lookup("chain").unwrap()) };
        let init: [i64; CHAIN_FIELDS] = std::array::from_fn(|i| (i as i64) * 1_000_003 - 7);
        let (mut got, mut want) = (init, init);
        // SAFETY: got has CHAIN_FIELDS elements.
        assert_eq!(unsafe { f(got.as_mut_ptr()) }, 0, "{opt:?}");
        assert_eq!(spike::chain_reference(&mut want, STMTS), Ok(()));
        assert_eq!(got, want, "{opt:?}");
    }
    assert!(
        sizes[1] < sizes[0],
        "Fast {} bytes, FastRa {}",
        sizes[0],
        sizes[1]
    );
}

thread_local! {
    static SUNK: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
}

extern "C" fn sink(p: *const i64, n: usize) {
    // SAFETY: the caller passes `n` readable words.
    SUNK.with(|s| *s.borrow_mut() = unsafe { std::slice::from_raw_parts(p, n) }.to_vec());
}

/// A stack buffer with thousands of stores through its address that escapes to a call (the
/// buffer a native frame's exits record registers in) goes through the pass pipeline and
/// the compiled function fills it: the passes that hide such a buffer give it back unchanged.
#[test]
fn a_big_escaping_buffer_survives_the_lean_passes() {
    const N: usize = 3000;
    for opt in [OptLevel::O2, OptLevel::Fast, OptLevel::FastRa] {
        let mut m = Module::new("buffer");
        spike::add_buffer(&m, N);
        m.verify().unwrap();
        m.optimize_as(opt, false).unwrap();
        m.verify().unwrap();
        let mut jit = Jit::new(opt, &runtime_symbols()).unwrap();
        jit.add_object(&m.emit(opt).unwrap()).unwrap();
        // SAFETY: the symbol is `extern "C" fn(i64, fn(*const i64)) -> i32`.
        let f: unsafe extern "C" fn(i64, extern "C" fn(*const i64)) -> i32 =
            unsafe { std::mem::transmute(jit.lookup("buffer").unwrap()) };
        extern "C" fn sink_n(p: *const i64) {
            sink(p, N);
        }
        // SAFETY: sink_n reads N words of the buffer.
        assert_eq!(unsafe { f(1000, sink_n) }, 0, "{opt:?}");
        let got = SUNK.with(|s| s.borrow().clone());
        let want: Vec<i64> = (0..N as i64).map(|k| 1000 + k).collect();
        assert_eq!(got, want, "{opt:?}");
    }
}
