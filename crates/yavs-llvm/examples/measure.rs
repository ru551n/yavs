//! JIT cost of one generated `chain` function, for the "too big for LLVM"
//! threshold (design rule 5). One size per process so peak RSS is clean:
//!
//! cargo run --release -p yavs-llvm --features llvm --example measure -- <stmts> [O0|O1|O2]
#![allow(unsafe_code)]

use std::time::Instant;

use yavs_llvm::spike::{self, CHAIN_FIELDS, ChainFn, OVERFLOW_CALLBACK};
use yavs_llvm::{Jit, Module, OptLevel};

extern "C" fn yavs_rt_overflow(_stmt: i64) {}

/// (VmRSS, VmHWM) in MiB, Linux only.
fn rss() -> (f64, f64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
            .map_or(f64::NAN, |kb| kb / 1024.0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let stmts: usize = args.next().map_or(800, |s| s.parse().expect("stmts"));
    let opt = match args.next().as_deref() {
        Some("O0") => OptLevel::O0,
        Some("O1") => OptLevel::O1,
        _ => OptLevel::O2,
    };
    let symbols = [(OVERFLOW_CALLBACK, yavs_rt_overflow as *const () as usize)];
    let mut jit = Jit::new(opt, &symbols).unwrap();
    let (rss0, _) = rss();

    let t = Instant::now();
    let mut m = Module::new("chain");
    spike::add_chain(&m, stmts);
    let insts = m.instruction_count();
    let t_build = t.elapsed();

    let t = Instant::now();
    m.optimize(opt).unwrap();
    let t_opt = t.elapsed();
    let insts_opt = m.instruction_count();

    let t = Instant::now();
    jit.add(m).unwrap();
    let addr = jit.lookup("chain").unwrap();
    let t_cg = t.elapsed();
    let (rss1, hwm) = rss();

    let mut state = [1i64; CHAIN_FIELDS];
    // SAFETY: chain has the ChainFn signature; state has CHAIN_FIELDS fields.
    let rc = unsafe { std::mem::transmute::<usize, ChainFn>(addr)(state.as_mut_ptr()) };
    assert_eq!(rc, 0);

    println!(
        "stmts={stmts} opt={opt:?} ir_insts={insts} ir_insts_after_opt={insts_opt} \
         build_ms={:.1} opt_ms={:.1} codegen_link_ms={:.1} total_ms={:.1} \
         rss_start_mib={rss0:.1} rss_end_mib={rss1:.1} peak_mib={hwm:.1} peak_delta_mib={:.1}",
        t_build.as_secs_f64() * 1e3,
        t_opt.as_secs_f64() * 1e3,
        t_cg.as_secs_f64() * 1e3,
        (t_build + t_opt + t_cg).as_secs_f64() * 1e3,
        hwm - rss0,
    );
}
