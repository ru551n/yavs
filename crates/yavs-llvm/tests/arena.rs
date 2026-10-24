//! The code arena (#679): the code of every JIT of the process lies in one contiguous region.
//! Run with `cargo test -p yavs-llvm --features llvm --test arena`.
#![cfg(feature = "llvm")]
#![allow(unsafe_code)] // calling JIT code is inherently unsafe

use yavs_llvm::spike::{self, CHAIN_FIELDS, ChainFn};
use yavs_llvm::{Jit, Module, OptLevel};

fn chain(stmts: usize) -> (Jit, usize) {
    let mut m = Module::new("arena");
    spike::add_chain(&m, stmts);
    m.verify().expect("IR verifies");
    m.optimize(OptLevel::O0).expect("passes run");
    let mut jit = Jit::new(OptLevel::O0, &[(spike::OVERFLOW_CALLBACK, 1)]).expect("LLJIT created");
    jit.add(m).expect("module added");
    let addr = jit.lookup("chain").expect("symbol");
    (jit, addr)
}

#[test]
fn jits_share_one_region_and_the_code_runs() {
    let (_a, fa) = chain(2000);
    let (_b, fb) = chain(3000);
    let Some([hot, huge, failed, overflow]) = yavs_llvm::code_arena_stats() else {
        // Not Linux x86_64, or `YAVS_LLVM_ARENA=0`: LLJIT's own linker, nothing to check.
        return;
    };
    eprintln!("arena: code {hot} huge chunks {huge} failed {failed} overflow {overflow}");
    assert!(hot > 0 && overflow == 0, "hot {hot} overflow {overflow}");
    // Two modules, one region: the functions are within the code zone (1 GiB) of each other.
    assert!(fa.abs_diff(fb) < (1 << 30), "{fa:#x} {fb:#x}");
    if huge > 0 {
        assert!(huge_kb(fa) > 0, "the chunk of {fa:#x} is not a huge page");
    }
    for (f, stmts) in [(fa, 2000), (fb, 3000)] {
        // SAFETY: `chain` has the ChainFn signature.
        let f: ChainFn = unsafe { std::mem::transmute(f) };
        let mut state = [0i64; CHAIN_FIELDS];
        let mut want = state;
        spike::chain_reference(&mut want, stmts).expect("reference");
        assert_eq!(unsafe { f(state.as_mut_ptr()) }, 0);
        assert_eq!(state, want);
    }
}

/// `AnonHugePages` (kB) of the mapping that holds `addr`, from `/proc/self/smaps`.
fn huge_kb(addr: usize) -> usize {
    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("smaps");
    let mut inside = false;
    for l in smaps.lines() {
        if let Some((range, _)) = l.split_once(' ')
            && let Some((a, b)) = range.split_once('-')
            && let (Ok(a), Ok(b)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16))
        {
            inside = (a..b).contains(&addr);
        } else if inside && let Some(v) = l.strip_prefix("AnonHugePages:") {
            return v.trim().trim_end_matches("kB").trim().parse().expect("kB");
        }
    }
    panic!("{addr:#x} is in no mapping");
}
