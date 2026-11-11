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

/// Print the counters when `YAVS_COUNTERS` is set; `s` is the kernel's.
pub fn report(s: yavs_kernel::Stats, rt: Option<&yavs_interp::Runtime>) {
    // (A measurement switch of the Cranelift tier, `YAVS_CLIFT_BLOCKCOV`: writes the blocks that ran.)
    #[cfg(feature = "clift")]
    yavs_clift::blockcov_write();
    if std::env::var_os("YAVS_COUNTERS").is_none() {
        return;
    }
    let (hits, runs, session_hits) = yavs_elab::call_cache_stats();
    let (reads, idle) = yavs_wave::var_counts();
    let (memo_hits, memo_runs) = yavs_interp::pmemo_stats();
    let mut rows = vec![
        ("cycles", s.cycles),
        ("process_runs", s.process_runs),
        ("single_runs", s.single_runs),
        ("pooled_batches", s.pooled_batches),
        ("threads_windows", s.threads_windows),
        ("estimates_skipped", s.estimates_skipped),
        ("estimates_cut", s.estimates_cut),
        ("threads_gained", s.threads_gained),
        ("wakes", s.wakes),
        ("idle_skips", s.idle_skips),
        ("lean_signals", s.lean_signals),
        ("bare_signals", s.bare_signals),
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
        ("gathers_copied", s.gathers_copied),
        ("gathers_read", s.gathers_read),
        ("scatter_spared", s.scatter_spared),
        ("image_writes", s.image_writes),
        ("images", s.images),
        ("image_bytes", s.image_bytes),
        ("copy_forwards", s.copy_forwards),
        ("wake_scans", s.wake_scans),
        ("wake_plans", s.wake_plans),
        ("ecall_hits", hits),
        ("ecall_runs", runs),
        ("ecall_session_hits", session_hits),
        ("opt_funcs", yavs_ir::optimised_functions()),
        ("dopes_scans", yavs_ir::opt::dopes_scans()),
        ("flat_elem_results", yavs_ir::flat_elem_results()),
        ("opt_huge", yavs_ir::opt::huge_bodies()),
        ("opt_bodies_kept", yavs_ir::opt::bodies_kept()),
        ("elab_restarts", yavs_ir::restarted_calls()),
        ("gc_collections", yavs_interp::gc_collections()),
        ("pmemo_hits", memo_hits),
        ("pmemo_runs", memo_runs),
        ("pmemo_off", yavs_interp::pmemo_off()),
        ("wave_var_reads", reads),
        ("wave_var_idle_skips", idle),
        (
            "units_stored",
            crate::cache::UNITS_STORED.load(std::sync::atomic::Ordering::Relaxed),
        ),
    ];
    // Only builds that count them (debug, or `op-census`).
    if let Some(n) = yavs_interp::ops_executed() {
        let before = OPS_BEFORE.load(std::sync::atomic::Ordering::Relaxed);
        rows.push(("ops_elab", before));
        rows.push(("ops", n - before));
        rows.push(("wake_sole_left", s.sole_left));
        rows.push(("block_copies", s.block_copies));
        rows.push(("whole_commits", s.whole_commits));
    }
    if let Some(n) = yavs_interp::lean_late_calls() {
        rows.push(("lean_late", n));
    }
    if let Some(n) = yavs_interp::booked_returns() {
        rows.push(("booked_returns", n));
    }
    if let Some(n) = yavs_interp::tier_skips() {
        rows.push(("tier_skips", n));
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
        rows.push(("spec_subs", c.subs as u64));
        rows.push(("spec_sub_ir", c.sub_ir));
        rows.push(("spec_sub_capped", c.sub_capped));
        rows.push(("spec_sub_gave_up", c.sub_gave_up));
        rows.push(("spec_sub_big", c.sub_big));
        rows.push(("spec_shared_ir", c.shared_ir));
        rows.push(("spec_copy_ir", c.copy_ir));
        rows.push(("retiers", rt.native_flags().retiers()));
        rows.push(("materialised", rt.native_flags().materialised_all()));
        let [
            asked,
            held,
            compiled,
            copies,
            withdrawn,
            deferred,
            later,
            waited,
            alongside,
            unrepaid,
            over_budget,
            unpaid_long,
            unlocked,
            tree_held,
            copy_calls,
        ] = rt.native_flags().count_stats();
        rows.push(("count_asked", asked));
        rows.push(("count_held", held));
        rows.push(("count_compiled", compiled));
        rows.push(("count_copies", copies));
        rows.push(("count_withdrawn", withdrawn));
        rows.push(("count_frame_deferred", deferred));
        rows.push(("count_frame_later", later));
        rows.push(("tier_waited", waited));
        rows.push(("tier_alongside", alongside));
        rows.push(("tier_unrepaid", unrepaid));
        rows.push(("tier_over_budget", over_budget));
        rows.push(("tier_unpaid_long", unpaid_long));
        rows.push(("tier_unlocked", unlocked));
        rows.push(("tier_tree_held", tree_held));
        rows.push(("tier_copy_calls", copy_calls));
    }
    // The calls of each native `ieee` subprogram that ran its general path (every build).
    let ieee: Vec<(String, u64)> = rt
        .map(|rt| yavs_interp::ieee_general(&rt.prog))
        .unwrap_or_default();
    // Copies of subprograms in Cranelift code: linked by a pre-key, lowered, compiled, and the id
    // symbols lowered code named.
    #[cfg(feature = "clift")]
    for (name, v) in [
        "clift_sub_linked",
        "clift_sub_lowered",
        "clift_sub_compiled",
        "clift_sub_ids",
    ]
    .into_iter()
    .zip(yavs_clift::sub_copy_stats())
    {
        rows.push((name, v as u64));
    }
    // Pre-key entries named by the Cranelift key the compile hashed (no second hash of the body).
    #[cfg(feature = "clift")]
    rows.push(("clift_key_reused", yavs_clift::key_reused() as u64));
    // Sites compiled code computes in place instead of calling the host: `is_x` or a reduction
    // of a vector, a `Cat` of bytes, a `LayoutOf` from the layout memo, a real to an integer, an
    // `ieee` op of two blocks, a gate that may meet a metavalue.
    #[cfg(feature = "clift")]
    for (name, v) in [
        "clift_scan_sites",
        "clift_cat_sites",
        "clift_layout_sites",
        "clift_real_int_sites",
        "clift_wide_sites",
        "clift_gate_sites",
    ]
    .into_iter()
    .zip(yavs_clift::inline_sites())
    {
        rows.push((name, v as u64));
    }
    #[cfg(feature = "clift")]
    rows.push(("clift_pre_unstored", yavs_clift::pre_unstored() as u64));
    for (name, v) in rows {
        eprintln!("counters: {name}={v}");
    }
    for (key, v) in ieee {
        eprintln!("counters: ieee_general.{key}={v}");
    }
    // The callbacks of compiled code, by kind (debug builds and `op-census`; both generators).
    #[cfg(feature = "clift")]
    for (name, v) in yavs_rt::callback_counts().into_iter().flatten() {
        eprintln!("counters: cb_{name}={v}");
    }
    // Skips of compiled drives held against the kernel's full test (`YAVS_INLCMP=check`).
    #[cfg(feature = "clift")]
    {
        let n = yavs_rt::rt::GATE_CHECKED.load(std::sync::atomic::Ordering::Relaxed);
        if n != 0 {
            eprintln!("counters: gate_checked={n}");
        }
        let n = yavs_rt::rt::DRIVE_SRC_HELD.load(std::sync::atomic::Ordering::Relaxed);
        if n != 0 {
            eprintln!("counters: drive_src_held={n}");
        }
        let n = yavs_rt::rt::SCATTER_CHECKED.load(std::sync::atomic::Ordering::Relaxed);
        if n != 0 {
            eprintln!("counters: scatter_checked={n}");
        }
        let n = yavs_rt::rt::GATHER_CHECKED.load(std::sync::atomic::Ordering::Relaxed);
        if n != 0 {
            eprintln!("counters: gather_checked={n}");
        }
        let n = yavs_rt::rt::HOLD_CHECKED.load(std::sync::atomic::Ordering::Relaxed);
        if n != 0 {
            eprintln!("counters: hold_checked={n}");
        }
    }
    // The calls into the host by instruction kind (`YAVS_COUNTERS=kinds|pc`, every build).
    for (name, v) in yavs_interp::tier_census() {
        eprintln!("counters: cb_{name}={v}");
    }
}
