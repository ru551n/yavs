//! `YAVS_COUNTERS=1`: the work counters of a run, one `counters: NAME=VALUE` line each on
//! stderr after `yavs run`. They count work, not time, so they are the same on every host and
//! run (with `--tier=interp --threads 1`): `crates/yavs/tests/perf.rs` checks them against
//! ceilings (root `AGENTS.md` section 4, "Work-counter tests"). `YAVS_COUNTERS=kinds` (or `pc`)
//! also splits the calls compiled code makes into the host by instruction kind (and function and
//! pc): `cb_<entry>.<Kind>` lines, `yavs_interp::tier_census`. A hidden test aid, not part of
//! the CLI contract.

static OPS_BEFORE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Called when the simulation is about to start: `ops` then counts the run, `ops_elab` what
/// elaboration interpreted before.
pub fn start() {
    let n = yavs_interp::ops_executed().unwrap_or(0);
    OPS_BEFORE.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// What `--tier=llvm` compiled: IR instructions inline, IR instructions the interpreter still
/// executes from compiled code, functions callable on native frames.
#[cfg(feature = "llvm")]
static LLVM: [std::sync::atomic::AtomicU64; 3] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 3];

#[cfg(feature = "llvm")]
pub fn llvm_compiled(inline: usize, interpreted: usize, native_frames: usize) {
    for (a, v) in LLVM.iter().zip([inline, interpreted, native_frames]) {
        a.store(v as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Print the counters when `YAVS_COUNTERS` is set; `s` is the kernel's.
pub fn report(s: yavs_kernel::Stats, rt: Option<&yavs_interp::Runtime>) {
    if std::env::var_os("YAVS_COUNTERS").is_none() {
        return;
    }
    let (hits, runs) = yavs_elab::call_cache_stats();
    let (reads, idle) = yavs_wave::var_counts();
    let (memo_hits, memo_runs) = yavs_interp::pmemo_stats();
    let mut rows = vec![
        ("cycles", s.cycles),
        ("process_runs", s.process_runs),
        ("single_runs", s.single_runs),
        ("pooled_batches", s.pooled_batches),
        ("wakes", s.wakes),
        ("idle_skips", s.idle_skips),
        ("lean_signals", s.lean_signals),
        ("no_code_signals", s.no_code_signals),
        ("skip_signals", s.skip_signals),
        ("signals", s.signals),
        ("events", s.events),
        ("update_fast", s.update_fast),
        ("update_timed", s.update_timed),
        ("update_general", s.update_general),
        ("quick_applied", s.quick_applied),
        ("direct_codes", s.direct_codes),
        ("direct_planes", s.direct_planes),
        ("direct_bytes", s.direct_bytes),
        ("general_assigns", s.general_assigns),
        ("copy_forwards", s.copy_forwards),
        ("ecall_hits", hits),
        ("ecall_runs", runs),
        ("opt_funcs", yavs_ir::optimised_functions()),
        ("elab_restarts", yavs_ir::restarted_calls()),
        ("gc_collections", yavs_interp::gc_collections()),
        ("pmemo_hits", memo_hits),
        ("pmemo_runs", memo_runs),
        ("pmemo_off", yavs_interp::pmemo_off()),
        ("wave_var_reads", reads),
        ("wave_var_idle_skips", idle),
    ];
    // Only builds that count them (debug, or `op-census`).
    if let Some(n) = yavs_interp::ops_executed() {
        let before = OPS_BEFORE.load(std::sync::atomic::Ordering::Relaxed);
        rows.push(("ops_elab", before));
        rows.push(("ops", n - before));
    }
    if let Some(rt) = rt {
        let c = rt.prog.copy_stats();
        rows.push(("spec_reserved", c.reserved as u64));
        rows.push(("spec_planned", c.planned));
        rows.push(("spec_copies", c.copies as u64));
        rows.push(("spec_switched", c.switched));
        rows.push(("spec_fresh", c.fresh as u64));
        rows.push(("spec_loaded", c.loaded as u64));
        rows.push(("spec_capped", c.capped));
        rows.push(("spec_full", c.full));
        rows.push(("spec_shared_ir", c.shared_ir));
        rows.push(("spec_copy_ir", c.copy_ir));
        rows.push(("retiers", rt.native_flags().retiers()));
    }
    #[cfg(feature = "llvm")]
    {
        let l = |i: usize| LLVM[i].load(std::sync::atomic::Ordering::Relaxed);
        rows.push(("llvm_inline_insts", l(0)));
        rows.push(("llvm_interpreted_insts", l(1)));
        rows.push(("llvm_native_frames", l(2)));
    }
    for (name, v) in rows {
        eprintln!("counters: {name}={v}");
    }
    // The callbacks of compiled code, by kind (debug builds and `op-census`; both generators).
    #[cfg(any(feature = "llvm", feature = "clift"))]
    for (name, v) in yavs_llvm::callback_counts().into_iter().flatten() {
        eprintln!("counters: cb_{name}={v}");
    }
    // The calls into the host by instruction kind (`YAVS_COUNTERS=kinds|pc`, every build).
    for (name, v) in yavs_interp::tier_census() {
        eprintln!("counters: cb_{name}={v}");
    }
}
