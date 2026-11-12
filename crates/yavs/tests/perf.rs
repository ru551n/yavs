//! Work-counter tests: the perf work of the kernel, interpreter and caches, guarded by counters
//! that do not depend on the host (`YAVS_COUNTERS=1`, `src/counters.rs`), not by timing.
//!
//! Each design under `perf/` is small, asserts its own result (a checksum in a `report`, so a
//! wrong optimisation that skips work cannot pass) and lists the counters that pin the path it is
//! about, each with a ceiling (`Max`) or, where fewer is the regression (a cache hit, a fast
//! path taken), a floor (`Min`). Runs use `--tier=interp --threads 1`: the same counts on every
//! run and platform. `ops` is counted in debug builds and with the `op-census` feature only.
//!
//! A failure names the counter, design, value and bound. To see every counter of every design:
//! `YAVS_PERF_PRINT=1 cargo test -p yavs --test perf -- --nocapture`. To update a bound, see root
//! `AGENTS.md` section 4, "Work-counter tests".

mod common;
use common::tempdir;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy)]
enum Bound {
    /// The counter must not exceed it.
    Max(u64),
    /// The counter must not fall below it.
    Min(u64),
}
use Bound::{Max, Min};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    counters: BTreeMap<String, u64>,
}

impl Run {
    fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_COUNTERS", "1")
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    let counters = stderr
        .lines()
        // (A key may hold `=`: the `ieee` operator keys.)
        .filter_map(|l| l.strip_prefix("counters: ")?.rsplit_once('='))
        .map(|(k, v)| (k.to_string(), v.parse().expect("counter value")))
        .collect();
    Run {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr,
        counters,
    }
}

/// Analyze `src` as `design.vhd` into `dir/lib` and run `design` under `--tier=<tier>` (`more`
/// after `--threads 1`).
fn run_design(dir: &Path, design: &str, src: &str, tier: &str, more: &[&str]) -> Run {
    run_design_env(dir, design, src, tier, more, &[])
}

/// `run_design` with environment variables set for the run.
fn run_design_env(
    dir: &Path,
    design: &str,
    src: &str,
    tier: &str,
    more: &[&str],
    env: &[(&str, &str)],
) -> Run {
    std::fs::write(dir.join(format!("{design}.vhd")), src).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let a = yavs(
        dir,
        &[],
        &["analyze", "--work", &work, &format!("{design}.vhd")],
    );
    assert_eq!(a.code, 0, "analyze {design}: {}", a.stderr);
    let tier = format!("--tier={tier}");
    let mut args = vec![
        "run",
        "--work",
        &work,
        &tier,
        "--no-cache",
        "--threads",
        "1",
    ];
    args.extend_from_slice(more);
    args.push(design);
    yavs(dir, env, &args)
}

/// What a bound violation says; `how` names where the counter comes from.
fn check_bounds(design: &str, r: &Run, bounds: &[(&str, Bound)]) {
    if std::env::var_os("YAVS_PERF_PRINT").is_some() {
        for (k, v) in &r.counters {
            println!("perf: {design} {k}={v}");
        }
    }
    let mut failed = Vec::new();
    for &(name, bound) in bounds {
        let Some(&v) = r.counters.get(name) else {
            // A plain release build without `op-census` collects no `ops`, `wake_sole_left`,
            // `lean_late` or `cb_*` counters; those bounds are skipped there. In a debug build they are
            // collected, so a missing counter is a failure, not a vacuous pass (#732).
            if cfg!(debug_assertions) {
                panic!("design `{design}`: no counter `{name}` in {:?}", r.counters);
            }
            assert!(
                name == "ops"
                    || name == "ops_elab"
                    || name == "wake_sole_left"
                    || name == "lean_late"
                    || name.starts_with("cb_"),
                "design `{design}`: no counter `{name}` in {:?}",
                r.counters
            );
            continue;
        };
        match bound {
            Max(m) if v > m => failed.push(format!(
                "counter `{name}` of design `{design}` is {v}, above its ceiling {m}: the design does more work than when the ceiling was set (a fast path lost, or a pass no longer applies)"
            )),
            Min(m) if v < m => failed.push(format!(
                "counter `{name}` of design `{design}` is {v}, below its floor {m}: a fast path or cache that used to be taken is not"
            )),
            _ => {}
        }
    }
    assert!(
        failed.is_empty(),
        "work-counter regression:\n  {}\n\nThe counters count work, not time: the same on every host. Find the change that made it\n\
         slower (`YAVS_PERF_PRINT=1 cargo test -p yavs --test perf -- --nocapture` prints all of them). If the new\n\
         value is intended, edit the bound in crates/yavs/tests/perf.rs (case `{design}`) to the new\n\
         value plus a small margin and say why in the commit message (root AGENTS.md section 4,\n\
         \"Work-counter tests\").",
        failed.join("\n  ")
    );
}

fn check(design: &str, src: &str, says: &str, tier: &str, more: &[&str], bounds: &[(&str, Bound)]) {
    let d = tempdir(design);
    let r = run_design(&d, design, src, tier, more);
    assert_eq!(r.code, 0, "{design}: {}", r.all());
    assert!(
        r.all().contains(says),
        "{design}: expected `{says}` in:\n{}",
        r.all()
    );
    check_bounds(design, &r, bounds);
}

/// One test per design file `perf/<name>.vhd`: the text its report must contain, extra `run`
/// arguments, and the bounds.
macro_rules! case {
    ($name:ident, $says:expr, $more:expr, [$(($c:literal, $b:expr)),* $(,)?]) => {
        #[test]
        fn $name() {
            check(
                stringify!($name),
                include_str!(concat!("perf/", stringify!($name), ".vhd")),
                $says,
                "interp",
                $more,
                &[$(($c, $b)),*],
            );
        }
    };
}

// The record variable is in the frame: the count is what its statements do without an object lookup.
case!(
    frame_vars,
    "n=200",
    &[],
    [("ops", Max(11200)), ("opt_type_memo", Min(1))]
);
// Slices between variables in the frame, one block: more facts than the optimiser's table keeps
// (#1059), so a table that stopped learning shows here as ops per statement.
case!(frame_slices, "x=", &[], [("ops", Max(11400))]);
// A function of 120 vectors with run-time bounds: the pass that reads descriptors in registers
// looks at each instruction a few times, not once per vector (a scan per vector made the
// optimiser cost grow with the square of the function).
case!(dopes_scan, "t=", &[], [("dopes_scans", Max(19000))]);
// A small function inlined at two levels: its optimised body is made once and handed out again,
// not optimised at each level (the floor shows a slide back).
case!(body_once, "s=672", &[], [("opt_bodies_kept", Min(1))]);
// A function of eight local vectors: whether the block of each copy is on a cycle is asked once
// per block and remembered for the pass (a walk of the function per copy made the optimiser grow
// with the square of a long function).
case!(cycle_memo, "m=", &[], [("opt_cycle_memo", Min(1))]);
// A function reading its string parameter's descriptor in the frame: its calls are lean from the
// 128th on (`Pre::late`), 2 x 300 calls, so about 470 lean ones.
case!(lean_late, "n=", &[], [("lean_late", Min(450))]);
// A function on an array of vectors called every cycle: its result is data and bounds (#1122), so
// each call skips the result pair and the descriptor copy; the floor shows a slide back to the pair.
case!(
    elem_result,
    "sum=8316",
    &[],
    [("flat_elem_results", Min(1)), ("ops", Max(75200))]
);
case!(
    pkg_table,
    "sum=212",
    &[],
    [("ops_elab", Max(480)), ("ops", Max(48))]
);
case!(
    elab_raw,
    "n=",
    &[],
    [
        ("opt_funcs", Max(3)),
        ("opt_funcs", Min(2)),
        ("ops_elab", Max(960))
    ]
);
case!(
    elab_restart,
    "n=8506 2",
    &[],
    [
        ("elab_restarts", Max(1)),
        ("elab_restarts", Min(1)),
        ("opt_funcs", Max(2)),
        ("opt_funcs", Min(2)),
        ("ops_elab", Max(60300)),
    ]
);

/// A call that has reported does not restart; one that has not does, and its report is printed once.
#[test]
fn elab_restart_report() {
    let d = tempdir("elab_restart_report");
    let r = run_design(
        &d,
        "elab_restart_report",
        include_str!("perf/elab_restart_report.vhd"),
        "interp",
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.all());
    let all = r.all();
    assert!(all.contains("n=3757 3757"), "{all}");
    for what in ["early report", "late report"] {
        assert_eq!(all.matches(what).count(), 1, "`{what}` in:\n{all}");
    }
    check_bounds(
        "elab_restart_report",
        &r,
        &[
            ("elab_restarts", Max(1)),
            ("elab_restarts", Min(1)),
            ("ops_elab", Max(113000)),
        ],
    );
}
case!(
    clk_toggle,
    "q=",
    &[],
    [
        ("cycles", Max(313)),
        ("process_runs", Max(317)),
        ("wakes", Max(314)),
        ("events", Max(313)),
        ("update_general", Max(2)),
        ("quick_applied", Min(194)),
        ("ops", Max(2943)),
    ]
);
case!(
    clk_together,
    "n=50,40,33,40",
    &[],
    [
        ("cycles", Max(350)),
        ("process_runs", Max(440)),
        ("update_general", Max(0)),
        ("update_timed", Min(185)),
        ("ops", Max(4647)),
    ]
);
case!(
    reg_planes,
    "c=",
    &[],
    [
        ("cycles", Max(313)),
        ("process_runs", Max(422)),
        ("wakes", Max(418)),
        ("update_general", Max(2)),
        ("direct_planes", Min(195)),
        ("ops", Max(4407)),
    ]
);
case!(
    wide_reg,
    "q=",
    &[],
    [
        ("cycles", Max(312)),
        ("update_general", Max(2)),
        ("direct_planes", Min(400)),
        ("ops", Max(3588)),
    ]
);
// A wide bus with no history attribute: compared and copied whole (`bare_signals`, a floor).
case!(
    bare_bus,
    "parts=",
    &[],
    [("bare_signals", Min(4)), ("lean_signals", Min(4))]
);
// Narrow registers nothing asks the history of keep none (#1111); `sq`, only the prefix of
// implicit signals, is lean and keeps its history.
case!(
    narrow_history,
    "dev=7 ded=4 k=165",
    &[],
    [("bare_signals", Min(9)), ("lean_signals", Min(10))]
);
case!(
    narrow_history_ext,
    "n=27 q=4006",
    &[],
    [("bare_signals", Min(2)), ("lean_signals", Min(2))]
);
// The records nothing asks the history of keep none (#1117): `p`, `po` and the packs of `pm`.
case!(
    record_history,
    "n=37663 m=4165 g=2400 p=377 e=6060 o=1944",
    &[],
    [("bare_signals", Min(9)), ("lean_signals", Min(9))]
);
case!(
    lean_events,
    "events=",
    &[],
    [
        ("cycles", Max(312)),
        ("lean_signals", Min(4)),
        ("update_general", Max(2)),
        ("ops", Max(13570)),
    ]
);
case!(
    elem_codes,
    "e=",
    &[],
    [
        ("cycles", Max(930)),
        ("no_code_signals", Min(2)),
        ("no_code_signals", Max(2)),
        ("ops", Max(26250)),
    ]
);
case!(
    reg_bytes,
    "n=",
    &[],
    [
        ("cycles", Max(313)),
        ("process_runs", Max(420)),
        ("wakes", Max(417)),
        ("update_general", Max(2)),
        ("direct_bytes", Min(195)),
        ("quick_applied", Min(194)),
        ("ops", Max(5552)),
    ]
);
case!(
    delayed,
    "a=",
    &[],
    [
        ("cycles", Max(739)),
        ("process_runs", Max(848)),
        ("wakes", Max(841)),
        ("update_general", Max(2)),
        ("quick_applied", Min(494)),
        ("ops", Max(6782)),
    ]
);
case!(
    shift_chain,
    "s=",
    &[],
    [
        ("cycles", Max(313)),
        ("process_runs", Max(1998)),
        ("wakes", Max(23)),
        ("direct_codes", Max(1770)),
        ("update_general", Max(2)),
        ("ops", Max(13130)),
    ]
);
case!(
    comb_chain,
    "n=",
    &[],
    [
        ("cycles", Max(1141)),
        ("process_runs", Max(1210)),
        ("wakes", Max(1181)),
        ("events", Max(1141)),
        ("update_general", Max(0)),
        ("ops", Max(870)),
    ]
);
// A whole record drive against the record's image: what holds its value makes no kernel call.
case!(
    rec_cmp,
    "n=44 i=7",
    &[],
    [
        ("scatter_spared", Min(6900)),
        ("image_writes", Max(305)),
        ("images", Min(1)),
    ]
);
// A record that changes in every leaf gives its image up at the first review.
case!(
    rec_cmp_drop,
    "i=1400 n=120",
    &[],
    [
        ("scatter_spared", Max(0)),
        ("image_writes", Max(5280)),
        ("images", Max(0)),
    ]
);
// The fields of a record sensitivity are one waiter, woken where the update finishes (#1069):
// 202 waiter-list scans before.
case!(
    rec_wake,
    "n=200 b=D001 d=7CD778B1",
    &[],
    [
        ("cycles", Max(1236)),
        ("wakes", Max(824)),
        ("events", Max(6481)),
        ("wake_scans", Max(4)),
    ]
);
case!(
    copy_chain,
    "c=",
    &[],
    [
        ("cycles", Max(1005)),
        ("process_runs", Max(1030)),
        ("update_general", Max(0)),
        ("ops", Max(465)),
        // The kernel commits the copies itself, outside the batch (#519).
        ("copy_forwards", Min(885)),
    ]
);
case!(
    copy_forward,
    "s=325679",
    &[],
    [
        ("cycles", Max(147)),
        ("process_runs", Max(277)),
        // Bus, slice, integer and record copies committed by the kernel (#519): 166.
        ("copy_forwards", Min(161)),
    ]
);
case!(
    gate_vec,
    "s=",
    &[],
    // The gates run in the kernel (#779): 10082 ops when they are interpreted.
    [("process_runs", Max(335)), ("ops", Max(5841))]
);
case!(
    mux_bus,
    "s=",
    &[],
    // The muxes, the relation and the gates run in the kernel (#786): 5094 ops when they are
    // interpreted (the rest is the stimulus process).
    [("process_runs", Max(257)), ("ops", Max(3340))]
);
case!(
    prog_bus,
    "s=",
    &[],
    // The tree, the conditions, the chain and the state test run in the kernel (#788): 7606 ops
    // when they are interpreted (the rest is the stimulus process).
    [("process_runs", Max(410)), ("ops", Max(2920))]
);
case!(
    fan,
    "hits=",
    &[],
    [
        ("cycles", Max(158)),
        ("process_runs", Max(1876)),
        ("wakes", Max(40)),
        ("update_general", Max(2)),
        ("ops", Max(8546)),
    ]
);
case!(
    lfsr_array,
    "acc=",
    &[],
    [
        ("cycles", Max(158)),
        ("process_runs", Max(537)),
        ("wakes", Max(527)),
        ("events", Max(1848)),
        ("update_general", Max(2)),
        ("direct_planes", Min(395)),
        ("ops", Max(9822)),
    ]
);
case!(
    lfsr_xor,
    "sum=",
    &[],
    [
        ("cycles", Max(155)),
        ("process_runs", Max(577)),
        ("wakes", Max(567)),
        ("events", Max(1846)),
        ("update_general", Max(0)),
        ("ops", Max(11834)),
    ]
);
case!(
    reduce_idiom,
    "n1=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(1374)),]
);
// Waiters whose long timeouts a signal beats (a message wait): a wake whose condition is false
// waits again with the queued timeout and the registered waiter (616 timeouts, 400 of them the
// stimulus's `wait for`, not 6817; 32 waiters, not 6416), the wake plan wakes them without a scan,
// and the heap holds the live timeouts and the ~200 of the waits that ended, not one per wake.
case!(
    timeout_stale,
    "s=1700 t=4010000000 fs",
    &[],
    [
        ("heap_peak", Max(223)),
        ("timeouts_queued", Max(635)),
        ("waiters_added", Max(33)),
        ("wake_scans", Max(2)),
    ]
);
case!(ieee_operands, "acc=", &[], [("ops", Max(2716))]);
case!(ieee_share, "n=", &[], [("ops", Max(3626))]);

// `perf/ieee_paths.vhd`: the calls per native `ieee` key that ran its general path, also under
// `clift`; an unlisted key is bounded by 0. A new path updates this table in the same commit
// (root `AGENTS.md` section 4, `docs/decisions.md` "Guards for the paths of the `ieee` natives").
const IEEE_GENERAL: &[(&str, u64)] = &[
    ("numeric_std.shift_right(signed,integer)signed", 120),
    (
        "std_logic_1164.to_01(std_ulogic_vector,std_ulogic)std_ulogic_vector",
        120,
    ),
    (
        "std_logic_1164.to_x01(std_ulogic_vector)std_ulogic_vector",
        60,
    ),
    (r#"std_logic_arith."-"(signed)signed"#, 60),
    (r#"std_logic_arith."abs"(signed)signed"#, 60),
    ("std_logic_arith.conv_signed(integer,integer)signed", 60),
    (
        "std_logic_arith.conv_std_logic_vector(integer,integer)std_ulogic_vector",
        60,
    ),
    (
        "std_logic_arith.conv_std_logic_vector(unsigned,integer)std_ulogic_vector",
        60,
    ),
    ("std_logic_arith.conv_unsigned(integer,integer)unsigned", 4),
    ("std_logic_arith.conv_unsigned(signed,integer)unsigned", 60),
    (
        "std_logic_arith.ext(std_ulogic_vector,integer)std_ulogic_vector",
        60,
    ),
    (
        "std_logic_arith.sxt(std_ulogic_vector,integer)std_ulogic_vector",
        60,
    ),
    (
        r#"std_logic_signed."+"(std_ulogic_vector)std_ulogic_vector"#,
        60,
    ),
    (
        r#"std_logic_signed."-"(std_ulogic_vector)std_ulogic_vector"#,
        60,
    ),
    (
        r#"std_logic_signed."abs"(std_ulogic_vector)std_ulogic_vector"#,
        60,
    ),
    (
        r#"std_logic_unsigned."+"(std_ulogic_vector)std_ulogic_vector"#,
        60,
    ),
];

/// What `perf/ieee_paths.vhd` reports: a checksum per width of every result.
const IEEE_PATHS_SAYS: [&str; 4] = [
    "W=128 sum=000000011EB213D5009266C302699A5C 480000064DDE232F87E48E73C10317F2 472738",
    "W=65 sum=15B4689C2C156CFB5 F9BD46E713B1AEF13 591689",
    "W=8 sum=D3 B6 796088",
    "W=128 syn sum=3FFFFFFFFFFFFFFF00000000007C6E92 568056",
];

/// `Max` bounds for every `<prefix><key>` counter of `r`: the count of `table`, 0 for a key not
/// listed. A listed key must be a counter of `r` (`check_bounds` fails on a missing one).
fn table_bounds(r: &Run, prefix: &str, table: &[(&str, u64)]) -> Vec<(String, Bound)> {
    let mut b: Vec<(String, Bound)> = r
        .counters
        .keys()
        .filter_map(|k| k.strip_prefix(prefix))
        .filter(|k| !table.iter().any(|(t, _)| t == k))
        .map(|k| (format!("{prefix}{k}"), Max(0)))
        .collect();
    b.extend(table.iter().map(|(k, v)| (format!("{prefix}{k}"), Max(*v))));
    b
}

fn ieee_paths_check(r: &Run, mut bounds: Vec<(String, Bound)>) {
    assert_eq!(r.code, 0, "ieee_paths: {}", r.all());
    for says in IEEE_PATHS_SAYS {
        assert!(
            r.stdout.contains(says),
            "ieee_paths: expected `{says}` in:\n{}",
            r.stdout
        );
    }
    let n = r
        .counters
        .keys()
        .filter(|k| k.starts_with("ieee_general."))
        .count();
    assert!(n > 100, "ieee_paths: {n} `ieee_general` counters");
    // The counter itself: a general path that is still taken.
    bounds.push((
        "ieee_general.std_logic_1164.to_01(std_ulogic_vector,std_ulogic)std_ulogic_vector".into(),
        Min(120),
    ));
    let b: Vec<(&str, Bound)> = bounds.iter().map(|(k, b)| (k.as_str(), *b)).collect();
    check_bounds("ieee_paths", r, &b);
}

#[test]
fn ieee_paths() {
    let d = tempdir("ieee_paths");
    let r = run_design(
        &d,
        "ieee_paths",
        include_str!("perf/ieee_paths.vhd"),
        "interp",
        &[],
    );
    let b = table_bounds(&r, "ieee_general.", IEEE_GENERAL);
    ieee_paths_check(&r, b);
}
case!(elem_loops, "acc=", &[], [("ops", Max(27715))]);
case!(wide_vec, "wide=0", &[], [("ops", Max(1997254))]);
case!(
    licm_loop,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(18376)),]
);
case!(
    slice_memo,
    "o=",
    &[],
    [
        ("cycles", Max(158)),
        ("process_runs", Max(323)),
        ("wakes", Max(317)),
        ("update_general", Max(2)),
        ("ops", Max(1427)),
    ]
);
case!(memo_wide, "q=64442", &[], [("ops", Max(75491))]);
// Pure functions on vectors called with repeating arguments (PoC ffdre, resize, mux): the memo of
// pure calls (#701) answers 436 of the 448 calls (ops 72600 without it).
case!(
    pure_memo,
    "q=9",
    &[],
    [
        ("pmemo_hits", Min(420)),
        ("pmemo_runs", Max(14)),
        ("ops", Max(15983))
    ]
);
// A cheap pure function (no loop, recursion, concatenation of bytes or native call but `ieee`
// gates and conversions) called with 40 different arguments, and a recursive one: the
// interpreter answers repeats of both from the memo (compiled callers ask it for the recursive
// one only, `clift::memo_loopfree`).
case!(memo_loopfree, "s=4190", &[], [("pmemo_hits", Min(740))]);
// A hex-digit parser of two 22-arm case functions (character to integer and to a nibble),
// 384 calls of each: the result tables (#722) read them inline, so none reaches the memo.
case!(
    case_tab,
    "sum=",
    &[],
    [
        ("pmemo_hits", Max(0)),
        ("pmemo_runs", Max(0)),
        ("ops", Max(10324))
    ]
);
// Two processes whose calls of a pure function never repeat: both give its memo up, which turns it
// off for all (#762: three votes were needed, so it stayed on the call path of compiled callers).
case!(
    memo_giveup,
    "acc=",
    &[],
    [
        ("pmemo_off", Min(1)),
        ("pmemo_runs", Max(130)),
        ("ops", Max(24067))
    ]
);
// The digit loop of a textio HRead (#744): a result-table nibble stored into the slice
// `sv(4 * i to 4 * i + 3)` and `is_x` on the same slice, 8 digits of each of 3 words, 16 times.
case!(hread_loop, "sum=", &[], [("ops", Max(4233))]);
// The same loop with an argument of an integer type, whose table read tests the range and a mask
// of valid digits (#769): the pair of the array result is not made on the table path.
case!(tab_tail, "sum=", &[], [("ops", Max(13549))]);
// Rows of bit vectors behind access values filled in nested loops by a procedure of the process
// (#768): the loads, the dereference and the descriptors leave the inner loop, which runs as one op.
case!(row_fill, "ones=", &[], [("ops", Max(4860))]);
case!(
    edge_forms,
    "edges=",
    &[],
    [
        ("cycles", Max(415)),
        ("process_runs", Max(1890)),
        ("wakes", Max(1870)),
        ("ops", Max(19143)),
    ]
);

// A `case` of 16 choices on a logic vector: one key and one dispatch, no compare per choice (#1051).
case!(case_vector, "n=8468", &[], [("ops", Max(37800))]);

// The forms of `edge_forms` through object aliases (#1001): the edge trigger of the signal itself,
// so each process runs on its edge only (2532 runs without it).
case!(
    edge_alias,
    "edges=",
    &[],
    [
        ("cycles", Max(415)),
        ("process_runs", Max(1680)),
        ("wakes", Max(1665)),
        ("ops", Max(16860)),
    ]
);

/// The same register written with each spelling of the clock edge costs the same work, the
/// `wait until` form included (#767): `ops` of the four are equal (the wait form skips the
/// first activation's edge test), with a ceiling for each.
#[test]
fn edge_cost_parity() {
    let forms = [
        (
            "rising_edge",
            "process (clk) begin if rising_edge(clk) then q(i) <= not q(i); end if; end process;",
        ),
        (
            "event",
            "process (clk) begin if clk'event and clk = '1' then q(i) <= not q(i); end if; end process;",
        ),
        (
            "stable",
            "process (clk) begin if not clk'stable and clk = '1' then q(i) <= not q(i); end if; end process;",
        ),
        (
            "wait_until",
            "process begin wait until rising_edge(clk); q(i) <= not q(i); end process;",
        ),
        // The same through an object alias (#1001).
        (
            "alias_rising_edge",
            "process (clk) alias c : std_logic is clk; begin if rising_edge(c) then q(i) <= not q(i); end if; end process;",
        ),
        (
            "alias_event",
            "process (clk) alias c : std_logic is clk; begin if c'event and clk = '1' then q(i) <= not q(i); end if; end process;",
        ),
        (
            "alias_stable",
            "process (clk) alias c : std_logic is clk; begin if not c'stable and c = '1' then q(i) <= not q(i); end if; end process;",
        ),
        (
            "alias_wait_until",
            "process alias c : std_logic is clk; begin wait until rising_edge(c); q(i) <= not q(i); end process;",
        ),
    ];
    let template = include_str!("perf/edge_cost.vhd");
    assert!(
        template.contains(forms[0].1),
        "the design holds the first form"
    );
    let mut ops = Vec::new();
    for (name, form) in forms {
        let d = tempdir(&format!("edge_cost_{name}"));
        let r = run_design(
            &d,
            "edge_cost",
            &template.replace(forms[0].1, form),
            "interp",
            &[],
        );
        assert_eq!(r.code, 0, "{name}: {}", r.all());
        assert!(r.all().contains("ones=8"), "{name}: {}", r.all());
        check_bounds(
            &format!("edge_cost/{name}"),
            &r,
            &[("process_runs", Max(1050)), ("ops", Max(8280))],
        );
        ops.push((name, r.counters.get("ops").copied()));
    }
    if let Some((first, Some(base))) = ops.first().copied() {
        for (name, n) in &ops {
            let n = n.expect("ops");
            assert!(
                n <= base && n + 8 >= base,
                "ops of {name} differ from {first}: {ops:?}"
            );
        }
    }
}

case!(
    wait_static,
    "s=",
    &[],
    [
        ("cycles", Max(414)),
        ("process_runs", Max(416)),
        ("wakes", Max(414)),
        ("update_general", Max(206)),
        ("events", Max(208)),
        ("ops", Max(7150)),
    ]
);
case!(
    inline_calls,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(2710)),]
);
// The message of a check is built after the branch that reads it: images, lengths and the
// concatenation stay out of the 270 passing calls (inlining of the check and `opt/sink.rs`).
case!(sink_msg, "failed=10 acc=14850", &[], [("ops", Max(6214))]);
// The defaults of a combinational process that a later drive deletes are not done on that path
// (`opt/drives.rs`): 303 scalar and 61 vector drives, 378 and 92 without the pass (the scalar
// drives that change nothing are skipped when they are made, `Ctx::assign_code_fast`, and not
// counted: 133 of them are committed; `ops` is the bound that sees the pass).
case!(
    drive_defaults,
    "sum=260",
    &[],
    [
        ("direct_codes", Max(137)),
        ("direct_planes", Max(63)),
        ("ops", Max(3564)),
    ]
);
// Repeated drives of inout ports (not direct) in one activation, mostly of the value held: the
// first leaves a skipped assignment, the repeats are skipped on the fast paths too (300 general
// assignments before, #720).
case!(
    drive_repeat,
    "sum=2712",
    &[],
    [("general_assigns", Max(2)), ("cycles", Max(515))]
);
// A run of scalar drives of variables that hold what the signals have is one op (`pre.rs`
// `drive_runs`); the drives that change are made, in the middle of a run too: 1811 direct
// code commits without the run, and the same cycles (the skipped drives keep their delta).
case!(
    drive_run,
    "ev=126 h=2002",
    &[],
    [
        ("cycles", Max(402)),
        ("cycles", Min(402)),
        ("direct_codes", Max(275)),
        ("ops", Max(15900)),
        // The wake plan (170 signals): 7 scans of waiter lists before, 2 now.
        ("wake_plans", Min(165)),
        ("wake_scans", Max(3)),
    ]
);
// Implicit signals in the design do not take the other signals off the fast update paths: only
// the updates in which a source of an implicit signal has activity, or one is due, run the general
// path (1342 of 1342 without this); the implicit signals still see every event of their sources.
case!(
    implicit_fast,
    "d_clk=300 d_slow=40 st=151 qu=41 tr=151",
    &[],
    [
        ("cycles", Max(1342)),
        ("cycles", Min(1342)),
        ("update_general", Max(380)),
        ("update_fast", Min(650)),
        ("ops", Max(22528)),
    ]
);
// Vector assignments that change nothing are skipped when they are made (605 direct plane
// commits without the skip), except behind an assignment of the same activation to the driver;
// one next to the vector entry before it joins it (175 without, #676).
case!(
    drive_vec,
    "ev=70 h=39957",
    &[],
    [
        ("cycles", Max(242)),
        ("cycles", Min(242)),
        ("direct_planes", Max(140)),
        ("ops", Max(18317)),
    ]
);
// Bits and slices of one register assigned in turn in one activation (the fields of a record) stay
// direct entries in order: no assignment goes through the driver's waveform (`update_general`).
// Parts next to the vector entry before them join it (1270 entries without, #676).
case!(
    mixed_entries,
    "ev=168 h=59167",
    &[],
    [
        ("cycles", Max(415)),
        ("update_general", Max(0)),
        ("direct_planes", Min(875)),
        ("direct_planes", Max(930)),
        ("direct_codes", Min(260)),
        ("events", Max(870)),
    ]
);
// A one-element aggregate choice is straight code and conversion copies run in the kernel. The
// signals with several resident waiters wake from their wake plan where the update finishes (394
// of them; 0 before, all through `cycle`).
case!(
    agg_carry,
    "acc=",
    &[],
    [
        ("ops", Max(6310)),
        ("wake_plans", Min(380)),
        ("wake_scans", Max(7)),
    ]
);
// An element of a signal parameter is read as that element of the actual, its bounds from the
// actual's descriptor: no read of the whole signal per element (#1121).
case!(sigparam_tree, "q=5CA0", &[], [("ops", Max(70700))]);
case!(
    static_chain,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(4974)),]
);
case!(
    record_sig,
    "addr=",
    &[],
    [
        ("cycles", Max(212)),
        ("process_runs", Max(216)),
        ("wakes", Max(212)),
        ("events", Max(650)),
        ("update_general", Max(2)),
        ("ops", Max(3785)),
        ("signals", Max(6)),
    ]
);
case!(
    sigrec_copy,
    "errors=0",
    &[],
    [
        ("cycles", Max(418)),
        ("process_runs", Max(525)),
        ("ops", Max(25669)),
        // The records `r` and `rin` are packs of leaves (#686), not one signal per leaf.
        ("signals", Max(20)),
    ]
);
case!(
    delta_ring,
    "n=",
    &[],
    [
        ("cycles", Max(394)),
        ("process_runs", Max(470)),
        ("wakes", Max(450)),
        ("events", Max(410)),
        ("update_general", Max(21)),
        ("ops", Max(923)),
    ]
);
// 600 `out` ports on elements of one array, each the only source of its element (collapsed):
// the multiple-sources check sorts the ranges of each leaf once (about 1800 steps, 180000 when it
// tested every pair) and the collapse searches the sorted sources of the shared leaf, 20 steps per
// port (360000 when it looked at every source of the leaf for every port).
case!(
    out_ports_shared,
    "sum=150446",
    &[],
    [("elab_source_steps", Max(14200)), ("signals", Max(602))]
);
// Record ports on elements of an array of records through two levels (`ch(i)`, `ch(i + 1)`),
// eight lanes: each formal is an alias of its element, so the lanes add no kernel signal, no
// port update and no second event (60 signals, 7498 events and 80 general updates when each
// formal was a signal of its own, copied per delta).
case!(
    port_elem_record,
    "sum=F54E",
    &[],
    [
        ("signals", Max(12)),
        ("events", Max(3640)),
        ("update_general", Max(0)),
        ("general_assigns", Max(0)),
        ("process_runs", Max(760)),
    ]
);
case!(
    out_port,
    "q=",
    &[],
    [
        ("cycles", Max(212)),
        ("process_runs", Max(216)),
        ("wakes", Max(212)),
        ("events", Max(224)),
        ("update_general", Max(2)),
        ("ops", Max(1660)),
    ]
);

case!(
    idle_regs,
    "q=42,28,70,0,70,44 b='1''1'",
    &[],
    [
        ("process_runs", Max(3100)),
        ("idle_skips", Min(4050)),
        ("ops", Max(29593)),
    ]
);

// Concurrent conditional assignments with arms that generics or a signal rule out skip the wakes
// those arms' signals cause (586 of 2715 activations).
case!(
    idle_when,
    "sum=1691",
    &[],
    [
        ("process_runs", Max(2200)),
        ("idle_skips", Min(560)),
        // Changed signals `wake_sole` leaves to `cycle`, which arms their readers once.
        ("wake_sole_left", Max(630)),
        ("ops", Max(27715)),
    ]
);
case!(
    idle_comb,
    "o=",
    &[],
    [
        ("process_runs", Max(4500)),
        ("idle_skips", Min(2850)),
        ("ops", Max(62766)),
    ]
);
// Variables assigned on every path before they are read (the arms of an `if`/`case`, the fields
// of a record variable one by one), and a `report` under a generic that is false in the instance:
// the processes are skipped (77 of 497 wakes); the instance where it is true reports every time,
// and a condition with an effect (an impure function) keeps its process running.
case!(
    idle_vars,
    "o=",
    &[],
    [
        ("process_runs", Max(433)),
        ("idle_skips", Min(75)),
        ("ops", Max(8324)),
    ]
);

// A `wait until` with a clock edge as a conjunct resumes its process only in a cycle with the
// edge (1507 process runs without).
case!(
    wait_edge,
    "hits=100 falls=100",
    &[],
    [
        ("process_runs", Max(1245)),
        ("wakes", Max(1245)),
        ("ops", Max(15225)),
    ]
);

/// Processes of instances with their own generics switch to copies specialised to their instance
/// data (`docs/design/specialisation.md`): the same report, fewer ops, three copies for the six
/// processes (the iterations of a generate and the instances with equal generics share one).
#[test]
fn spec_copies() {
    let d = tempdir("spec_copies");
    let src = include_str!("perf/spec_copies.vhd");
    let run = |env: &[(&str, &str)]| run_design_env(&d, "spec_copies", src, "interp", &[], env);
    let (shared, copies) = (
        run(&[("YAVS_NO_SPEC", "1")]),
        run(&[("YAVS_SPEC_AFTER", "2")]),
    );
    assert_eq!((shared.code, copies.code), (0, 0), "{}", copies.all());
    assert!(copies.all().contains("sum="), "{}", copies.all());
    assert_eq!(shared.stdout, copies.stdout);
    assert_eq!(shared.counters["spec_switched"], 0);
    check_bounds(
        "spec_copies",
        &copies,
        &[
            ("spec_copies", Max(3)),
            ("spec_switched", Min(6)),
            ("spec_capped", Max(0)),
            ("spec_full", Max(0)),
            ("ops", Max(33700)),
        ],
    );
}

/// The frame a process's declarative part makes (record, array-of-record and unconstrained array
/// variables with bounds from generics, constants, a value a function made) is known to the
/// copy, which folds the layouts and bounds it reads (#678): the same report, fewer ops.
#[test]
fn spec_record_var() {
    let d = tempdir("spec_record_var");
    let src = include_str!("perf/spec_record_var.vhd");
    let run = |env: &[(&str, &str)]| run_design_env(&d, "spec_record_var", src, "interp", &[], env);
    let (shared, copies) = (
        run(&[("YAVS_NO_SPEC", "1")]),
        run(&[("YAVS_SPEC_AFTER", "2")]),
    );
    assert_eq!((shared.code, copies.code), (0, 0), "{}", copies.all());
    assert!(copies.all().contains("sum="), "{}", copies.all());
    assert_eq!(shared.stdout, copies.stdout);
    check_bounds(
        "spec_record_var",
        &copies,
        &[("spec_switched", Min(3)), ("ops", Max(127900))],
    );
}

/// A variable after an array-of-composite variable, passed by reference: the copy does not fold it
/// (#856).
#[test]
fn spec_comp_slot() {
    let d = tempdir("spec_comp_slot");
    let src = include_str!("perf/spec_comp_slot.vhd");
    let run = |env: &[(&str, &str)]| run_design_env(&d, "spec_comp_slot", src, "interp", &[], env);
    let (shared, copies) = (
        run(&[("YAVS_NO_SPEC", "1")]),
        run(&[("YAVS_SPEC_AFTER", "2")]),
    );
    assert_eq!((shared.code, copies.code), (0, 0), "{}", copies.all());
    assert!(copies.all().contains("k=27"), "{}", copies.all());
    check_bounds("spec_comp_slot", &copies, &[("spec_switched", Min(1))]);
}

/// A process with subprograms nested in it, holding an access object and a line (so it runs on the
/// serial path), switches to a copy too: the nested subprograms reach the frame the copy keeps,
/// and the copy folds the layouts they do not write (#721): the same report, fewer ops.
#[test]
fn spec_nested() {
    let d = tempdir("spec_nested");
    let src = include_str!("perf/spec_nested.vhd");
    let run = |env: &[(&str, &str)]| run_design_env(&d, "spec_nested", src, "interp", &[], env);
    let (shared, copies) = (
        run(&[("YAVS_NO_SPEC", "1")]),
        run(&[("YAVS_SPEC_AFTER", "2")]),
    );
    assert_eq!((shared.code, copies.code), (0, 0), "{}", copies.all());
    assert!(copies.all().contains("sum="), "{}", copies.all());
    assert_eq!(shared.stdout, copies.stdout);
    // `ops` is counted in debug and `op-census` builds only (`check_bounds` asks it there).
    if let (Some(c), Some(s)) = (copies.counters.get("ops"), shared.counters.get("ops")) {
        assert!(c < s, "{:?}", copies.counters);
    }
    check_bounds(
        "spec_nested",
        &copies,
        &[("spec_switched", Min(3)), ("ops", Max(224_200))],
    );
}

/// A memory-model process: a nested procedure indexes an array variable of the process and writes
/// a variable the body drives a signal from. Its constant-offset addresses into the frame are
/// followed, not taken as escapes, so the copy still has the facts of its declarative part
/// (#1063): the same report, fewer ops.
#[test]
fn spec_nested_mem() {
    let d = tempdir("spec_nested_mem");
    let src = include_str!("perf/spec_nested_mem.vhd");
    let run = |env: &[(&str, &str)]| run_design_env(&d, "spec_nested_mem", src, "interp", &[], env);
    let (shared, copies) = (
        run(&[("YAVS_NO_SPEC", "1")]),
        run(&[("YAVS_SPEC_AFTER", "2")]),
    );
    assert_eq!((shared.code, copies.code), (0, 0), "{}", copies.all());
    assert!(copies.all().contains("sum="), "{}", copies.all());
    assert_eq!(shared.stdout, copies.stdout);
    check_bounds(
        "spec_nested_mem",
        &copies,
        // Without the facts: 52 425 ops (shared code 56 077); the margin is tighter than 3 percent so
        // that a lost fact shows.
        &[("spec_switched", Min(2)), ("ops", Max(51_800))],
    );
}

/// The idle skip changes nothing a run shows: the same output (values, times and deltas of the
/// reports) with it off, on and checked (a skipped activation run and compared), and it skips.
/// `idle_regs`: clocked processes, `idle_comb`: combinational ones, clocked concurrent
/// assignments and variables.
#[test]
fn idle_skip_changes_nothing() {
    let designs = [
        ("idle_regs", include_str!("perf/idle_regs.vhd"), 4000),
        ("idle_comb", include_str!("perf/idle_comb.vhd"), 2700),
        ("idle_when", include_str!("perf/idle_when.vhd"), 100),
        ("idle_vars", include_str!("perf/idle_vars.vhd"), 60),
    ];
    for (design, src, floor) in designs {
        let d = tempdir(&format!("idle_equal_{design}"));
        let run = |mode: &str| {
            run_design_env(&d, design, src, "interp", &[], &[("YAVS_IDLE_SKIP", mode)])
        };
        let (off, on, check) = (run("off"), run("on"), run("check"));
        assert_eq!(
            (off.code, on.code, check.code),
            (0, 0, 0),
            "{design}: {}",
            check.all()
        );
        assert_eq!(off.stdout, on.stdout, "{design}");
        assert_eq!(off.stdout, check.stdout, "{design}");
        assert_eq!(off.counters["idle_skips"], 0, "{design}");
        assert!(
            on.counters["idle_skips"] > floor,
            "{design}: the skip did not fire: {:?}",
            on.counters
        );
        assert_eq!(
            on.counters["idle_skips"], check.counters["idle_skips"],
            "{design}"
        );
        // Coverage (user, 2026-10-24): counters count activations, so the skip is off with
        // them; toggle coverage alone keeps it.
        let cov = |kinds: &str| {
            let kinds = format!("--cov-kind={kinds}");
            run_design(&d, design, src, "interp", &["--cov-file=c.ycov", &kinds])
        };
        let (counted, toggle) = (cov("statement,branch,toggle"), cov("toggle"));
        assert_eq!((counted.code, toggle.code), (0, 0), "{design}");
        assert_eq!(counted.stdout, on.stdout, "{design}");
        assert_eq!(toggle.stdout, on.stdout, "{design}");
        assert_eq!(counted.counters["idle_skips"], 0, "{design}");
        assert_eq!(
            toggle.counters["idle_skips"], on.counters["idle_skips"],
            "{design}"
        );
    }
}

/// A wide bus that keeps no history gives the report of one that keeps it.
#[test]
fn no_history_changes_nothing() {
    let d = tempdir("bare_equal");
    let src = include_str!("perf/bare_bus.vhd");
    let run = |mode: &str| {
        run_design_env(
            &d,
            "bare_bus",
            src,
            "interp",
            &[],
            &[("YAVS_NO_HISTORY", mode)],
        )
    };
    let (off, on) = (run("off"), run("on"));
    assert_eq!((off.code, on.code), (0, 0), "{}", on.all());
    assert!(on.stdout.contains("parts="), "{}", on.all());
    assert_eq!(off.stdout, on.stdout);
    assert_eq!(off.counters["bare_signals"], 0);
    // The two buses, and `n` and `done`, narrow (#1111).
    assert_eq!(on.counters["bare_signals"], 4, "{:?}", on.counters);
    assert_eq!(off.counters["events"], on.counters["events"]);
    // Coverage observes the run: every signal keeps its history.
    let cov = run_design_env(&d, "bare_bus", src, "interp", &["--cov=sbt"], &[]);
    assert_eq!(cov.code, 0, "{}", cov.all());
    assert_eq!(cov.counters["bare_signals"], 0);
}

/// Wide signals that keep only each block's last event give the report of the full history.
#[test]
fn lean_events_change_nothing() {
    let d = tempdir("lean_equal");
    let src = include_str!("perf/lean_events.vhd");
    let run = |mode: &str| {
        run_design_env(
            &d,
            "lean_events",
            src,
            "interp",
            &[],
            &[("YAVS_LEAN_EVENTS", mode)],
        )
    };
    let (off, on) = (run("off"), run("on"));
    assert_eq!((off.code, on.code), (0, 0), "{}", on.all());
    assert!(on.stdout.contains("events="), "{}", on.all());
    assert_eq!(off.stdout, on.stdout);
    assert_eq!(off.counters["lean_signals"], 0);
    // `a` and `b`, and the narrow `n` and `done`, which nothing asks the history of (#1111).
    assert_eq!(on.counters["lean_signals"], 4, "{:?}", on.counters);
    // Coverage observes the run: every signal keeps its history.
    let cov = run_design_env(&d, "lean_events", src, "interp", &["--cov=sbt"], &[]);
    assert_eq!(cov.code, 0, "{}", cov.all());
    assert_eq!(cov.stdout, on.stdout);
    assert_eq!(cov.counters["lean_signals"], 0);
}

/// Signals nothing reads a single element of keep no byte codes and give the report of the
/// ones that keep them.
#[test]
fn byte_codes_change_nothing() {
    let d = tempdir("codes_equal");
    let src = include_str!("perf/elem_codes.vhd");
    let run = |mode: &str, tier: &str| {
        run_design_env(&d, "elem_codes", src, tier, &[], &[("YAVS_CODES", mode)])
    };
    let tiers: &[&str] = &["interp"];
    for &tier in tiers {
        let (all, some) = (run("all", tier), run("", tier));
        assert_eq!((all.code, some.code), (0, 0), "{}", some.all());
        assert!(some.stdout.contains("e="), "{}", some.all());
        assert_eq!(all.stdout, some.stdout, "{tier}");
        assert_eq!(all.counters["no_code_signals"], 0);
        assert_eq!(some.counters["no_code_signals"], 2, "{:?}", some.counters);
    }
}

/// A port-map actual, a sensitivity entry and a signal passed to a subprogram that reads no
/// element of it are no element reads (#734): `b`, `c`, `e` and `h` of the design keep no codes.
#[test]
fn connections_and_sensitivity_need_no_byte_codes() {
    let d = tempdir("codes_conn");
    let src = include_str!("perf/elem_codes_conn.vhd");
    let run = |mode: &str, tier: &str| {
        run_design_env(
            &d,
            "elem_codes_conn",
            src,
            tier,
            &[],
            &[("YAVS_CODES", mode)],
        )
    };
    let tiers: &[&str] = &["interp"];
    for &tier in tiers {
        let (all, some) = (run("all", tier), run("", tier));
        assert_eq!((all.code, some.code), (0, 0), "{}", some.all());
        assert!(some.stdout.contains("hits="), "{}", some.all());
        assert_eq!(all.stdout, some.stdout, "{tier}");
        assert_eq!(all.counters["no_code_signals"], 0);
        assert_eq!(some.counters["no_code_signals"], 4, "{:?}", some.counters);
    }
}

/// An assignment of the value a signal holds is dropped only when nothing can observe the
/// signal's activity (user, 2026-10-24): `skip_signals` counts the signals that allow it. Each
/// observer of activity (a plugin, a debugger, `'active`, `'last_active`, `'transaction`,
/// `'quiet`) takes it away; a waveform file records values only, and coverage keeps the skip
/// (it changes no count: user, 2026-10-24, #707).
#[test]
fn skipping_follows_observers() {
    let d = tempdir("skip_observers");
    let src = include_str!("perf/skip_observers.vhd");
    let run = |decl: &str, extra: &str, more: &[&str]| {
        let src = src.replace("--EXTRA_DECL", decl).replace("--EXTRA", extra);
        let r = run_design(&d, "skip_observers", &src, "interp", more);
        assert_eq!(r.code, 0, "{}", r.all());
        r.counters["skip_signals"]
    };
    // a, b and n, and nothing else.
    assert_eq!(run("", "", &[]), 3);
    // A waveform records values, not activity.
    let fst = d.join("w.fst");
    let wave = format!("--wave={}", fst.display());
    assert_eq!(run("", "", &[&wave]), 3);
    // Coverage of every kind: the skip changes no count.
    assert_eq!(run("", "", &["--cov=sbt", "--cov-file=c.ycov"]), 3);
    assert_eq!(run("", "", &["--cov-file=c.ycov", "--cov-kind=toggle"]), 3);
    assert_eq!(
        run(
            "",
            "",
            &[
                "--cov-file=c.ycov",
                "--cov-kind=statement,branch,condition,expression,toggle"
            ]
        ),
        3
    );
    // `'active` and `'last_active` anywhere switch it off for the whole design.
    assert_eq!(
        run(
            "",
            "process (a) begin assert a'active or true; end process;",
            &[]
        ),
        0
    );
    assert_eq!(
        run(
            "",
            "process (a) begin assert a'last_active >= 0 ns; end process;",
            &[]
        ),
        0
    );
    // An implicit signal makes its prefix an observer (`'transaction`, `'quiet`): that signal
    // and the implicit one are out, the others stay.
    assert_eq!(
        run("", "process (a'transaction) begin null; end process;", &[]),
        2
    );
    assert_eq!(run("signal q : boolean;", "q <= a'quiet(5 ns);", &[]), 3);
}

/// The third run of a design in one library (the first notes the closure, the second stores, the
/// third hits the cache).
#[test]
fn ecall_cache() {
    let d = tempdir("ecall_cache");
    let work = format!("work:{}", d.join("lib").display());
    for (f, t) in [
        ("ecall_pkg.vhd", include_str!("perf/ecall_pkg.vhd")),
        ("ecall_tb.vhd", include_str!("perf/ecall_tb.vhd")),
    ] {
        std::fs::write(d.join(f), t).expect("write");
        let a = yavs(&d, &[], &["analyze", "--work", &work, f]);
        assert_eq!(a.code, 0, "{f}: {}", a.stderr);
    }
    let run = || {
        yavs(
            &d,
            &[],
            &[
                "run",
                "--work",
                &work,
                "--tier=interp",
                "--threads",
                "1",
                "ecall_tb",
            ],
        )
    };
    let (first, _, third) = (run(), run(), run());
    // Cold: every call runs; warm: none does. The answer is the same.
    check_bounds(
        "ecall_cache (cold run)",
        &first,
        &[("ecall_hits", Max(0)), ("ecall_runs", Min(3))],
    );
    assert!(third.all().contains("c=43 106 253"), "{}", third.all());
    check_bounds(
        "ecall_cache (third run)",
        &third,
        &[("ecall_hits", Min(3)), ("ecall_runs", Max(0))],
    );
}

/// How many `debug: units:` notes of the run contain `what`.
fn unit_notes(r: &Run, what: &str) -> u64 {
    r.stderr
        .lines()
        .filter(|l| l.starts_with("debug: units: ") && l.contains(what))
        .count() as u64
}

/// Three dependent files analysed one per call, then again: every unit of the second round is
/// verified against its entry (nothing is replayed) and no file is parsed.
#[test]
fn unit_cache() {
    let d = tempdir("unit_cache");
    let work = format!("work:{}", d.join("lib").display());
    let files = [
        ("unit_pkg.vhd", include_str!("perf/unit_pkg.vhd")),
        ("unit_mid.vhd", include_str!("perf/unit_mid.vhd")),
        ("unit_tb.vhd", include_str!("perf/unit_tb.vhd")),
    ];
    for (f, t) in files {
        std::fs::write(d.join(f), t).expect("write");
    }
    let round = || -> Vec<Run> {
        files
            .iter()
            .map(|(f, _)| {
                let r = yavs(
                    &d,
                    &[("YAVS_LOG", "debug")],
                    &["analyze", "--work", &work, f],
                );
                assert_eq!(r.code, 0, "{f}: {}", r.stderr);
                r
            })
            .collect()
    };
    let cold = round();
    let warm = round();
    let sum = |rs: &[Run], what| rs.iter().map(|r| unit_notes(r, what)).sum::<u64>();
    let counters: BTreeMap<String, u64> = [
        ("cold_stores", sum(&cold, "store")),
        ("warm_hits", sum(&warm, "hit")),
        ("warm_verified", sum(&warm, "verified")),
        ("warm_misses", sum(&warm, "miss")),
        ("warm_stores", sum(&warm, "store")),
        ("warm_files_not_parsed", sum(&warm, "not parsed")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let r = Run {
        code: 0,
        stdout: String::new(),
        stderr: String::new(),
        counters,
    };
    check_bounds(
        "unit_cache",
        &r,
        &[
            // A warm file whose units report nothing is verified, not loaded (#581): nothing is replayed.
            ("warm_hits", Max(0)),
            ("warm_verified", Min(3)),
            ("warm_misses", Max(0)),
            ("warm_stores", Max(0)),
            ("warm_files_not_parsed", Min(3)),
        ],
    );
    let run = yavs(
        &d,
        &[],
        &["run", "--work", &work, "--tier=interp", "unit_tb"],
    );
    assert!(run.all().contains("K=7 M=8 f=28"), "{}", run.all());
}

/// `--wave-variables`: a process that did not run since the last read costs one check, not one
/// per variable.
#[test]
fn wave_vars() {
    let d = tempdir("wave_vars");
    let fst = d.join("w.fst");
    let more = [
        format!("--wave={}", fst.display()),
        "--wave-variables".to_string(),
    ];
    let more: Vec<&str> = more.iter().map(String::as_str).collect();
    let r = run_design(
        &d,
        "wave_vars",
        include_str!("perf/wave_vars.vhd"),
        "interp",
        &more,
    );
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.all().contains("done"), "{}", r.all());
    check_bounds(
        "wave_vars",
        &r,
        &[
            ("wave_var_reads", Max(478)),
            ("wave_var_idle_skips", Min(242)),
        ],
    );
}

/// Under the Cranelift tier (`--tier=clift`, compiled before the run): the callbacks compiled code
/// makes, on the designs of the LLVM cases above. Run by the `clift` job of CI
/// (`cargo test -p yavs --features clift`, debug build: the callbacks are counted).
#[cfg(feature = "clift")]
mod clift {
    use super::*;

    macro_rules! clift_case {
        ($name:ident, $design:literal, $says:expr, [$(($c:literal, $b:expr)),* $(,)?]) => {
            #[test]
            fn $name() {
                check(
                    $design,
                    include_str!(concat!("perf/", $design, ".vhd")),
                    $says,
                    "clift",
                    &[],
                    &[$(($c, $b)),*],
                );
            }
        };
    }

    // A string concatenation in a native frame is one call of `yavs_rt_cat`, not a step.
    clift_case!(
        cat_bytes,
        "cat_bytes",
        "s=",
        [
            ("cb_xstep", Max(0)),
            ("cb_cat", Min(194)),
            ("cb_cat", Max(206))
        ]
    );
    // A layout from a run-time descriptor on a native frame is one call of the layout memo, not
    // a step (400 calls; the process's own `LayoutOf`s stay steps).
    clift_case!(
        layout_into,
        "layout_into",
        "s=34398",
        [
            ("cb_xstep", Max(0)),
            ("cb_layout_into", Min(390)),
            ("cb_layout_into", Max(410))
        ]
    );
    // A deallocation on a native frame is one call of the host's free, not a step (the `new`
    // before it stays one).
    clift_case!(
        free_native,
        "free_native",
        "s=",
        [
            ("cb_xstep", Max(410)),
            ("cb_free", Min(390)),
            ("cb_free", Max(410))
        ]
    );
    // A real rounded to an integer is computed in compiled code, on a native frame and in frame
    // mode, not stepped.
    clift_case!(
        real_int,
        "real_int",
        "s=",
        [("cb_xstep", Max(0)), ("cb_step", Max(4))]
    );
    // A product of 65 by 65 elements, a sum of 128, reductions and concatenations of 65 and gates
    // of vectors holding `U`, all in place: no native call and no step of a `Cat` (the callbacks
    // left are the waits and the report).
    clift_case!(
        wide_ieee,
        "wide_ieee",
        "n=",
        [
            ("cb_native", Max(2)),
            ("cb_signal", Max(207)),
            ("clift_wide_sites", Min(4))
        ]
    );
    // Slices of a run-time length returned by small functions (bounds from generics and from
    // the arguments) are copied in place on their native frames: no call of the runtime's copy,
    // which each of the 400 calls made before.
    clift_case!(
        slice_len,
        "clift_slice_len",
        "rows=",
        [("cb_lcopy", Max(0)), ("clift_lcopy_count_sites", Min(2))]
    );
    // A by-reference address of the enclosing frame in frame-mode code: no callback per access.
    clift_case!(
        nested_byref,
        "clift_nested_byref",
        "c=400",
        [("cb_host", Max(20))]
    );
    // `uniform` is computed in place by this tier too: no native call for it.
    clift_case!(
        uniform,
        "llvm_uniform",
        "acc=",
        [("cb_native", Max(4)), ("cb_host", Max(2))]
    );
    clift_case!(
        clocked,
        "llvm_clocked",
        "q=",
        [
            // Every activation through the tier's own entry (`rt::direct_entry`).
            ("cb_direct", Min(940)),
            ("cb_signal", Max(5)),
            ("cb_step", Max(8)),
            ("cb_native", Max(5)),
            ("cb_drive", Max(998)),
            ("cb_drive", Min(900)),
            ("cb_wait", Max(19)),
            ("cb_wait_same", Max(1018)),
        ]
    );
    clift_case!(
        sigread,
        "llvm_sigread",
        "wide=",
        [
            // A short read in one plane word: the codes' cold way never runs.
            ("cb_rare_codes", Max(0)),
            // No block is lowered as never run without `YAVS_CLIFT_CHEAP`.
            ("cb_steps", Max(0)),
            ("cb_signal", Max(60)),
            ("cb_step", Max(9)),
            ("cb_native", Max(56)),
            // Whole values from an operator are driven from where they are (`yavs_rt_drive_src`).
            ("cb_drive", Max(160)),
            ("cb_drive", Min(140)),
            ("cb_drive_src", Min(95)),
            ("cb_lcopy", Max(55)),
        ]
    );
    clift_case!(
        edge_forms,
        "edge_forms",
        "edges=",
        [
            ("cb_signal", Max(8)),
            // `WaitEdge` goes to the kernel itself (`yavs_rt_wait_edge`, #766), no step; the
            // `wait until <edge>` waits take `yavs_rt_wait_same` too (#772).
            ("cb_step", Max(8)),
            ("cb_drive", Max(1867)),
            ("cb_drive", Min(1700)),
            ("cb_wait_same", Max(1780)),
        ]
    );
    // A `wait until .. for` that waits again: the `WaitAt` and its `Suspend` are one call of the
    // kernel (`yavs_rt_wait_at`) and `TimedOut` another, no step and no signal callback (6400
    // wakes; 13 k steps and 6.8 k signal callbacks before).
    clift_case!(
        timed_wait,
        "timeout_stale",
        "s=1700 t=4010000000 fs",
        [
            ("cb_step", Max(207)),
            ("cb_signal", Max(431)),
            ("cb_wait_at", Max(6600)),
            ("cb_timed_out", Max(6610)),
        ]
    );
    // The memo of pure calls (#701) answers the compiled calls of this tier too; once a function
    // passed its first 64 calls, compiled code finds a repeat in its table by itself (316 of the
    // 433 hits) and calls the host only for the rest.
    clift_case!(
        pure_memo,
        "pure_memo",
        "q=9",
        [
            ("pmemo_hits", Min(420)),
            ("pmemo_runs", Max(14)),
            ("pmemo_inline_hits", Min(305)),
            ("cb_memo", Max(145)),
            ("cb_memo", Min(130)),
        ]
    );
    // A compiled call of a cheap pure function skips the memo: its body costs less than the
    // probe (`MemoSig::dear`); one of a recursive function without a loop does not.
    clift_case!(
        memo_loopfree,
        "memo_loopfree",
        "s=4190",
        [
            // `half` (recursive) only: `pick` would add its 248 hits.
            ("pmemo_hits", Max(525)),
            ("pmemo_hits", Min(495)),
            ("pmemo_inline_hits", Min(460)),
            ("cb_memo", Max(270)),
            ("clift_memo_unprobed_sites", Min(1)),
        ]
    );
    // Functions and procedures that read signals run on native frames (#519): 900 calls, no step.
    clift_case!(
        sigfun,
        "llvm_sigfun",
        "acc=",
        [("cb_step", Max(4)), ("cb_signal", Max(4))]
    );
    // A process without a wait runs its jumps back in compiled code, leaving every 2^20th (#583).
    clift_case!(
        nowait,
        "llvm_nowait",
        "jumps=",
        [("cb_step", Max(4)), ("cb_signal", Max(2))]
    );
    // One call of the layout memo per field instead of a step of the `LayoutOf` (#679).
    clift_case!(
        layout_field,
        "llvm_layout_field",
        "acc=38682",
        [("cb_step", Max(5)), ("cb_layout", Max(3400))]
    );
    // Zero-delay scalar drives kept for one host call (`Batch`); kept drives of unchanged
    // elements make no kernel entry.
    clift_case!(
        drive_batch,
        "llvm_drive_batch",
        "q=",
        [("cb_drive", Max(158)), ("direct_codes", Max(107))]
    );
    // Element loops of native frames planned in the code (#757). The fills call out; the row
    // copies (one plane block each side) are inline: a copy that calls out again raises it.
    clift_case!(
        eloop,
        "llvm_eloop",
        "acc=",
        [
            ("cb_eloop", Min(20)),
            ("cb_eloop", Max(20)),
            ("cb_step", Max(6)),
            ("cb_deopt", Max(0))
        ]
    );
    // Whole-record reads and drives (`SigGather`, `SigScatter`) go through the signal callback,
    // not a step (#790).
    clift_case!(
        record_signals,
        "sigrec_copy",
        "errors=0",
        [
            ("cb_step", Max(4)),
            // A gather copied or a held scatter compared in compiled code needs no callback; a
            // scatter that misses the compare is compared run by run in `yavs_rt_scatter_cmp`.
            ("cb_signal", Max(72)),
            ("cb_signal", Min(66)),
            ("cb_record_cmp", Min(160)),
            ("cb_record_cmp", Max(175)),
        ]
    );
    // Records that change in every leaf at every clock: once the review has let their images go,
    // compiled code drives them through `yavs_rt_scatter`, on the kernel itself, not the signal
    // callback and the host's plan (#1110), and reads these records of vectors in place, leaf by
    // leaf: a read through `yavs_rt_gather` raises `cb_record` and `gathers_read`.
    clift_case!(
        rec_direct,
        "rec_direct",
        "q=0028F00281",
        [
            ("cb_record", Min(1000)),
            ("cb_record", Max(1100)),
            // Before the review lets the images go, the misses of the compare.
            ("cb_record_cmp", Min(3950)),
            ("cb_signal", Max(2800)),
            ("gathers_read", Max(40)),
        ]
    );
    // A procedure with signal parameters of an array type runs on native frames: the element
    // references sliced inline (no `yavs_rt_xstep`), its 16-element reads in place, no frame
    // materialised (#1121).
    clift_case!(
        sigparam_tree,
        "sigparam_tree",
        "q=5CA0",
        [
            ("cb_xstep", Max(4)),
            ("cb_step", Max(4)),
            ("cb_signal", Max(6)),
            ("cb_deopt", Max(2)),
            ("cb_planes", Max(4)),
        ]
    );
    clift_case!(
        reg_planes,
        "reg_planes",
        "c=100",
        [
            ("cb_signal", Max(5)),
            ("cb_step", Max(10)),
            ("cb_drive", Max(212)),
            ("cb_drive", Min(190)),
            ("cb_drive_src", Min(190)),
            ("cb_native", Max(4)),
        ]
    );

    // Elements of an array of `signed` in a function (#1122): the copies of the operands and of
    // the result are inline, the local starts zeroed with no fill, the parameter's region in the
    // caller's frame is found at the entry without a call.
    clift_case!(
        elem_ops,
        "clift_elem_ops",
        "sum=",
        [
            ("cb_lcopy", Max(0)),
            ("cb_xstep", Max(1)),
            ("cb_region", Max(0))
        ]
    );

    /// Copies of subprograms (the corpus test `subcopy_shapes`, copies of everything) warm, the
    /// last run with other copy ids (`YAVS_COPY_SKIP`): their code names the ids through id
    /// symbols, so it is all linked and none lowered.
    #[test]
    fn sub_copies_link_under_other_ids() {
        let d = tempdir("sub_copy_ids");
        let work = format!("work:{}", d.join("lib").display());
        let src = include_str!("../../../tests/corpus/composite/subcopy_shapes.vhd");
        std::fs::write(d.join("t.vhd"), src).expect("write");
        let a = yavs(
            &d,
            &[],
            &["analyze", "--std=2008", "--work", &work, "t.vhd"],
        );
        assert_eq!(a.code, 0, "{}", a.stderr);
        let run = |env: &[(&str, &str)]| {
            let mut e = vec![("YAVS_SPEC_AFTER", "2"), ("YAVS_SPEC_ALL", "1")];
            e.extend(env);
            let r = yavs(
                &d,
                &e,
                &[
                    "run",
                    "--std=2008",
                    "--work",
                    &work,
                    "--tier=clift-auto",
                    "--tier-heat=1,0,0",
                    "--threads",
                    "1",
                    "subcopy_shapes",
                ],
            );
            assert_eq!(r.code, 0, "{}", r.all());
            r
        };
        let cold = run(&[]);
        check_bounds(
            "sub_copy_ids (cold)",
            &cold,
            &[("clift_sub_lowered", Min(41)), ("clift_sub_ids", Min(51))],
        );
        run(&[]);
        let moved = run(&[("YAVS_COPY_SKIP", "2")]);
        assert_eq!(moved.stdout, cold.stdout);
        // (The empty slots count among the copies: every id moved by 2.)
        assert_eq!(
            moved.counters["spec_copies"],
            cold.counters["spec_copies"] + 2
        );
        check_bounds(
            "sub_copy_ids (warm, other ids)",
            &moved,
            &[
                ("clift_sub_linked", Min(41)),
                ("clift_sub_lowered", Max(0)),
                ("clift_sub_compiled", Max(0)),
            ],
        );
    }

    // `perf/ieee_paths.vhd` compiled: native calls per key (`cb_native` of `YAVS_COUNTERS=pc`), an
    // unlisted key 0 (computed in place on every call); the in-place ops are listed for their calls
    // on more than 64 elements. A new path updates this table in the same commit.
    const IEEE_NATIVE_CALLS: &[(&str, u64)] = &[
        (r#"numeric_std."*"(integer,signed)signed"#, 87),
        (r#"numeric_std."*"(integer,unsigned)unsigned"#, 87),
        (r#"numeric_std."*"(signed,integer)signed"#, 87),
        (r#"numeric_std."*"(signed,signed)signed"#, 174),
        (r#"numeric_std."*"(unsigned,integer)unsigned"#, 87),
        (r#"numeric_std."*"(unsigned,unsigned)unsigned"#, 174),
        (r#"numeric_std."+"(integer,signed)signed"#, 58),
        (r#"numeric_std."+"(integer,unsigned)unsigned"#, 58),
        (r#"numeric_std."+"(signed,integer)signed"#, 58),
        (r#"numeric_std."+"(signed,signed)signed"#, 58),
        (r#"numeric_std."+"(unsigned,integer)unsigned"#, 116),
        (r#"numeric_std."+"(unsigned,unsigned)unsigned"#, 174),
        (r#"numeric_std."-"(integer,signed)signed"#, 58),
        (r#"numeric_std."-"(integer,unsigned)unsigned"#, 58),
        (r#"numeric_std."-"(signed)signed"#, 116),
        (r#"numeric_std."-"(signed,integer)signed"#, 58),
        (r#"numeric_std."-"(signed,signed)signed"#, 58),
        (r#"numeric_std."-"(unsigned,integer)unsigned"#, 58),
        (r#"numeric_std."-"(unsigned,unsigned)unsigned"#, 116),
        (r#"numeric_std."/"(integer,unsigned)unsigned"#, 58),
        (r#"numeric_std."/"(signed,integer)signed"#, 58),
        (r#"numeric_std."/"(signed,signed)signed"#, 58),
        (r#"numeric_std."/"(unsigned,integer)unsigned"#, 58),
        (r#"numeric_std."/"(unsigned,unsigned)unsigned"#, 58),
        (r#"numeric_std."/="(signed,signed)boolean"#, 60),
        (r#"numeric_std."/="(unsigned,integer)boolean"#, 60),
        (r#"numeric_std."<"(signed,signed)boolean"#, 60),
        (r#"numeric_std."<"(unsigned,unsigned)boolean"#, 58),
        (r#"numeric_std."<="(unsigned,unsigned)boolean"#, 60),
        (r#"numeric_std."="(integer,signed)boolean"#, 60),
        (r#"numeric_std."="(unsigned,unsigned)boolean"#, 60),
        (r#"numeric_std.">"(signed,integer)boolean"#, 60),
        (r#"numeric_std.">"(unsigned,integer)boolean"#, 60),
        (r#"numeric_std.">="(integer,unsigned)boolean"#, 60),
        (r#"numeric_std.">="(signed,signed)boolean"#, 60),
        (r#"numeric_std."?/="(unsigned,unsigned)std_ulogic"#, 60),
        (r#"numeric_std."?<"(unsigned,unsigned)std_ulogic"#, 60),
        (r#"numeric_std."?<="(signed,signed)std_ulogic"#, 60),
        (r#"numeric_std."?="(unsigned,unsigned)std_ulogic"#, 60),
        (r#"numeric_std."?>"(signed,integer)std_ulogic"#, 60),
        (r#"numeric_std."?>="(unsigned,integer)std_ulogic"#, 60),
        (r#"numeric_std."abs"(signed)signed"#, 116),
        (r#"numeric_std."and"(signed)std_ulogic"#, 62),
        (r#"numeric_std."and"(signed,signed)signed"#, 60),
        (r#"numeric_std."and"(unsigned,unsigned)unsigned"#, 60),
        (r#"numeric_std."mod"(integer,unsigned)unsigned"#, 58),
        (r#"numeric_std."mod"(signed,integer)signed"#, 58),
        (r#"numeric_std."mod"(signed,signed)signed"#, 58),
        (r#"numeric_std."mod"(unsigned,integer)unsigned"#, 58),
        (r#"numeric_std."mod"(unsigned,unsigned)unsigned"#, 58),
        (r#"numeric_std."nand"(unsigned,unsigned)unsigned"#, 60),
        (r#"numeric_std."nor"(unsigned,unsigned)unsigned"#, 60),
        (r#"numeric_std."not"(signed)signed"#, 60),
        (r#"numeric_std."not"(unsigned)unsigned"#, 60),
        (r#"numeric_std."or"(unsigned,unsigned)unsigned"#, 118),
        (r#"numeric_std."rem"(integer,unsigned)unsigned"#, 58),
        (r#"numeric_std."rem"(signed,integer)signed"#, 58),
        (r#"numeric_std."rem"(signed,signed)signed"#, 58),
        (r#"numeric_std."rem"(unsigned,integer)unsigned"#, 58),
        (r#"numeric_std."rem"(unsigned,unsigned)unsigned"#, 58),
        (r#"numeric_std."xnor"(unsigned,unsigned)unsigned"#, 60),
        (r#"numeric_std."xor"(signed,signed)signed"#, 1820),
        (r#"numeric_std."xor"(unsigned)std_ulogic"#, 62),
        (r#"numeric_std."xor"(unsigned,unsigned)unsigned"#, 2836),
        ("numeric_std.maximum(signed,signed)signed", 60),
        ("numeric_std.maximum(unsigned,integer)unsigned", 60),
        ("numeric_std.maximum(unsigned,unsigned)unsigned", 60),
        ("numeric_std.minimum(signed,integer)signed", 60),
        ("numeric_std.minimum(signed,signed)signed", 60),
        ("numeric_std.minimum(unsigned,unsigned)unsigned", 60),
        ("numeric_std.resize(signed,integer)signed", 180),
        ("numeric_std.resize(unsigned,integer)unsigned", 534),
        ("numeric_std.rotate_left(unsigned,integer)unsigned", 118),
        ("numeric_std.rotate_right(signed,integer)signed", 60),
        ("numeric_std.rotate_right(unsigned,integer)unsigned", 60),
        ("numeric_std.shift_left(signed,integer)signed", 60),
        ("numeric_std.shift_left(unsigned,integer)unsigned", 60),
        ("numeric_std.shift_right(signed,integer)signed", 150),
        ("numeric_std.shift_right(unsigned,integer)unsigned", 178),
        ("numeric_std.std_match(unsigned,unsigned)boolean", 120),
        ("numeric_std.to_integer(signed)integer", 60),
        ("numeric_std.to_integer(unsigned)integer", 60),
        ("numeric_std.to_signed(integer,integer)signed", 60),
        ("numeric_std.to_unsigned(integer,integer)unsigned", 118),
        (
            r#"std_logic_1164."?/="(std_ulogic_vector,std_ulogic_vector)std_ulogic"#,
            60,
        ),
        (
            r#"std_logic_1164."?="(std_ulogic_vector,std_ulogic_vector)std_ulogic"#,
            60,
        ),
        (
            r#"std_logic_1164."and"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_1164."not"(std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_1164."or"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_1164."xor"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            2387,
        ),
        ("std_logic_1164.is_x(std_ulogic_vector)boolean", 62),
        (
            "std_logic_1164.to_bitvector(std_ulogic_vector,bit)bit_vector",
            150,
        ),
        ("std_logic_1164.to_hstring(std_ulogic_vector)string", 5),
        (
            "std_logic_1164.to_x01(std_ulogic_vector)std_ulogic_vector",
            150,
        ),
        (r#"std_logic_arith."*"(signed,signed)signed"#, 87),
        (r#"std_logic_arith."*"(unsigned,unsigned)unsigned"#, 87),
        (r#"std_logic_arith."+"(signed,signed)signed"#, 58),
        (r#"std_logic_arith."+"(std_ulogic,signed)signed"#, 60),
        (r#"std_logic_arith."+"(unsigned,integer)unsigned"#, 174),
        (r#"std_logic_arith."+"(unsigned,std_ulogic)unsigned"#, 60),
        (r#"std_logic_arith."+"(unsigned,unsigned)unsigned"#, 116),
        (r#"std_logic_arith."-"(integer,unsigned)unsigned"#, 58),
        (r#"std_logic_arith."-"(signed)signed"#, 60),
        (r#"std_logic_arith."-"(signed,integer)signed"#, 58),
        (r#"std_logic_arith."-"(signed,std_ulogic)signed"#, 60),
        (r#"std_logic_arith."-"(std_ulogic,unsigned)unsigned"#, 60),
        (r#"std_logic_arith."-"(unsigned,unsigned)unsigned"#, 116),
        (r#"std_logic_arith."/="(signed,signed)boolean"#, 60),
        (r#"std_logic_arith."<"(unsigned,unsigned)boolean"#, 58),
        (r#"std_logic_arith."<="(signed,signed)boolean"#, 60),
        (r#"std_logic_arith."="(unsigned,unsigned)boolean"#, 60),
        (r#"std_logic_arith.">"(signed,integer)boolean"#, 60),
        (r#"std_logic_arith.">="(unsigned,unsigned)boolean"#, 60),
        (r#"std_logic_arith."abs"(signed)signed"#, 60),
        ("std_logic_arith.conv_signed(integer,integer)signed", 60),
        (
            "std_logic_arith.conv_std_logic_vector(integer,integer)std_ulogic_vector",
            60,
        ),
        (
            "std_logic_arith.conv_std_logic_vector(unsigned,integer)std_ulogic_vector",
            60,
        ),
        ("std_logic_arith.conv_unsigned(signed,integer)unsigned", 60),
        (
            "std_logic_arith.ext(std_ulogic_vector,integer)std_ulogic_vector",
            60,
        ),
        ("std_logic_arith.shl(unsigned,unsigned)unsigned", 60),
        ("std_logic_arith.shr(signed,unsigned)signed", 60),
        (
            "std_logic_arith.sxt(std_ulogic_vector,integer)std_ulogic_vector",
            60,
        ),
        (
            r#"std_logic_signed."+"(std_ulogic,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."+"(std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."+"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."-"(std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."-"(std_ulogic_vector,integer)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."-"(std_ulogic_vector,std_ulogic)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_signed."<"(std_ulogic_vector,std_ulogic_vector)boolean"#,
            60,
        ),
        (
            r#"std_logic_signed."abs"(std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."*"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            90,
        ),
        (
            r#"std_logic_unsigned."+"(std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."+"(std_ulogic_vector,integer)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."+"(std_ulogic_vector,std_ulogic)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."+"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."-"(integer,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."-"(std_ulogic,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."-"(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector"#,
            60,
        ),
        (
            r#"std_logic_unsigned."<"(std_ulogic_vector,std_ulogic_vector)boolean"#,
            60,
        ),
        (
            r#"std_logic_unsigned."="(std_ulogic_vector,std_ulogic_vector)boolean"#,
            60,
        ),
        (
            r#"std_logic_unsigned.">"(std_ulogic_vector,integer)boolean"#,
            90,
        ),
        (
            "std_logic_unsigned.shl(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector",
            60,
        ),
        (
            "std_logic_unsigned.shr(std_ulogic_vector,std_ulogic_vector)std_ulogic_vector",
            60,
        ),
    ];

    // The whole drives of the 112-element `l1` of `clift_elem_ops` are the kernel's from compiled
    // code (`yavs_interp::direct_drive_wide`), none through the host (200 before).
    #[test]
    fn elem_ops_wide_drives() {
        let d = tempdir("clift_elem_ops_kinds");
        let r = run_design_env(
            &d,
            "clift_elem_ops",
            include_str!("perf/clift_elem_ops.vhd"),
            "clift",
            &[],
            &[("YAVS_COUNTERS", "kinds")],
        );
        assert_eq!(r.code, 0, "clift_elem_ops: {}", r.all());
        assert!(
            r.counters.contains_key("cb_drive_bit.Drive"),
            "{:?}",
            r.counters
        );
        assert_eq!(
            r.counters.get("cb_drive_wide.Drive"),
            None,
            "wide drives through the host"
        );
    }

    #[test]
    fn ieee_paths() {
        let d = tempdir("clift_ieee_paths");
        let mut r = run_design_env(
            &d,
            "ieee_paths",
            include_str!("perf/ieee_paths.vhd"),
            "clift",
            &["--verbose"],
            &[("YAVS_COUNTERS", "pc")],
        );
        assert!(
            !r.stderr.contains("stays interpreted"),
            "ieee_paths: {}",
            r.stderr
        );
        let mut calls: BTreeMap<String, u64> = IEEE_NATIVE_CALLS
            .iter()
            .map(|(k, _)| (format!("ieee_calls.{k}"), 0))
            .collect();
        for (k, v) in &r.counters {
            if let Some(rest) = k.strip_prefix("cb_native.yavs.ieee.") {
                let key = rest.rsplit_once(".pc").map_or(rest, |p| p.0);
                let key = key.split_once('@').map_or(key, |p| p.0);
                *calls.entry(format!("ieee_calls.{key}")).or_insert(0) += v;
            }
        }
        r.counters.extend(calls);
        let mut b = table_bounds(&r, "ieee_general.", IEEE_GENERAL);
        b.extend(table_bounds(&r, "ieee_calls.", IEEE_NATIVE_CALLS));
        // The in-place arms' cold native call (metavalues, widths, lengths): declines that
        // grew mean an arm lost its case.
        b.push(("cb_rare_native".into(), Max(17960)));
        ieee_paths_check(&r, b);
    }
}

/// A pure function that reads a signal outside it (a warning under `--relaxed`) is not a function of
/// its arguments: the idle skip must not trust it. The process that calls it runs again when that
/// signal changes, with the skip off, on and checked.
#[test]
fn idle_skip_ignores_impure_pure_function() {
    let d = tempdir("idle_relaxed");
    std::fs::write(d.join("idle_relaxed_reads.vhd"), IDLE_RELAXED).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &[
            "analyze",
            "--relaxed",
            "--work",
            &work,
            "idle_relaxed_reads.vhd",
        ],
    );
    assert_eq!(a.code, 0, "analyze: {}", a.stderr);
    for mode in ["off", "on", "check"] {
        let r = yavs(
            &d,
            &[("YAVS_IDLE_SKIP", mode)],
            &[
                "run",
                "--relaxed",
                "--work",
                &work,
                "--tier=interp",
                "--no-cache",
                "idle_relaxed_reads",
            ],
        );
        assert_eq!(r.code, 0, "{mode}: {}", r.all());
        let last = r.stdout.lines().rfind(|l| l.contains("o="));
        assert!(
            last.is_some_and(|l| l.ends_with("o='1'")),
            "{mode}: the last report is {last:?}: {}",
            r.all()
        );
    }
}

/// Not under `perf/`: other tests analyse every design there without `--relaxed`.
const IDLE_RELAXED: &str = r#"-- A pure function that reads a signal outside it (--relaxed makes that a warning) is not a function
-- of its arguments: a process that calls it runs again when that signal changes, though the signals
-- it names have no event (the idle skip must not trust it).
library ieee;
use ieee.std_logic_1164.all;

package pk is
  signal pxs : std_ulogic := '0';
  function f(x : std_ulogic) return std_ulogic;
end package;

package body pk is
  function f(x : std_ulogic) return std_ulogic is
  begin
    return x and pxs;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pk.all;

entity idle_relaxed_reads is end;

architecture t of idle_relaxed_reads is
  signal a, o, c : std_ulogic := '0';
begin
  p : process (a, c)
  begin
    o <= f(a);
  end process;

  stim : process
  begin
    a <= '1';
    wait for 1 ns;
    for i in 1 to 10 loop
      c <= not c;
      wait for 1 ns;
    end loop;
    report "o=" & std_ulogic'image(o);
    pxs <= '1';
    c <= not c;
    wait for 1 ns;
    report "o=" & std_ulogic'image(o);
    wait;
  end process;
end architecture;
"#;
