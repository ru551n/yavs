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
        .filter_map(|l| l.strip_prefix("counters: ")?.split_once('='))
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
            // A plain release build without `op-census` collects no `ops` or `cb_*`
            // counters; those bounds are skipped there. In a debug build they are
            // collected, so a missing counter is a failure, not a vacuous pass (#732).
            if cfg!(debug_assertions) {
                panic!("design `{design}`: no counter `{name}` in {:?}", r.counters);
            }
            assert!(
                name == "ops" || name == "ops_elab" || name.starts_with("cb_"),
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
case!(
    lean_events,
    "events=",
    &[],
    [
        ("cycles", Max(312)),
        ("lean_signals", Min(2)),
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
    [("process_runs", Max(2)), ("ops", Max(1481)),]
);
case!(ieee_operands, "acc=", &[], [("ops", Max(2716))]);
case!(ieee_share, "n=", &[], [("ops", Max(3626))]);
case!(elem_loops, "acc=", &[], [("ops", Max(27715))]);
case!(wide_vec, "wide=0", &[], [("ops", Max(2143000))]);
case!(
    licm_loop,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(19800)),]
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
// A hex-digit parser of two 22-arm case functions (character to integer and to a nibble),
// 384 calls of each: the result tables (#722) read them inline, so none reaches the memo.
case!(
    case_tab,
    "sum=",
    &[],
    [
        ("pmemo_hits", Max(0)),
        ("pmemo_runs", Max(0)),
        ("ops", Max(11920))
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
case!(tab_tail, "sum=", &[], [("ops", Max(14590))]);
// Rows of bit vectors behind access values filled in nested loops by a procedure of the process
// (#768): the loads, the dereference and the descriptors leave the inner loop, which runs as one op.
case!(row_fill, "ones=", &[], [("ops", Max(5290))]);
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
        ("ops", Max(19173)),
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
// A one-element aggregate choice is straight code and conversion copies run in the kernel.
case!(agg_carry, "acc=", &[], [("ops", Max(6310))]);
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
        ("ops", Max(26932)),
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
        &[("spec_switched", Min(3)), ("ops", Max(163500))],
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
    assert!(
        copies.counters["ops"] < shared.counters["ops"],
        "{:?}",
        copies.counters
    );
    check_bounds(
        "spec_nested",
        &copies,
        &[("spec_switched", Min(3)), ("ops", Max(224_200))],
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
            run_design(&d, design, src, "interp", &["--cov=c.ycov", &kinds])
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
    assert_eq!(on.counters["lean_signals"], 2, "{:?}", on.counters);
    // Coverage observes the run: every signal keeps its history.
    let cov = run_design_env(&d, "lean_events", src, "interp", &["--cov"], &[]);
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
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["interp", "auto"]
    } else {
        &["interp"]
    };
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
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["interp", "auto"]
    } else {
        &["interp"]
    };
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
    assert_eq!(run("", "", &["--cov=c.ycov"]), 3);
    assert_eq!(run("", "", &["--cov=c.ycov", "--cov-kind=toggle"]), 3);
    assert_eq!(
        run(
            "",
            "",
            &[
                "--cov=c.ycov",
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

/// Under the LLVM tier (`--tier=llvm`, compiled before the run, so the same every time): the
/// callbacks compiled code makes and what it leaves to the interpreter. Run by the `llvm` job of
/// CI (`cargo test -p yavs --features llvm`).
#[cfg(feature = "llvm")]
mod llvm {
    use super::*;

    macro_rules! llvm_case {
        ($name:ident, $says:expr, [$(($c:literal, $b:expr)),* $(,)?]) => {
            #[test]
            fn $name() {
                check(
                    stringify!($name),
                    include_str!(concat!("perf/", stringify!($name), ".vhd")),
                    $says,
                    "llvm",
                    &[],
                    &[$(($c, $b)),*],
                );
            }
        };
    }

    llvm_case!(
        llvm_sigread,
        "wide=",
        [
            ("cb_signal", Max(56)),
            ("cb_step", Max(12)),
            ("cb_native", Max(56)),
            ("cb_drive", Max(263)),
            ("cb_troot", Max(53)),
            ("cb_endstmt", Max(53)),
            ("cb_lcopy", Max(53)),
            ("cb_fast", Max(1)),
            ("llvm_inline_insts", Min(586)),
            ("llvm_interpreted_insts", Max(50)),
            ("llvm_native_frames", Min(1)),
        ]
    );
    llvm_case!(
        llvm_frames,
        "acc=76800",
        [
            ("cb_signal", Max(2)),
            ("cb_step", Max(4)),
            ("cb_native", Max(2)),
            ("llvm_inline_insts", Min(157)),
            ("llvm_interpreted_insts", Max(11)),
            ("llvm_native_frames", Min(4)),
        ]
    );
    llvm_case!(
        llvm_wide_fill,
        "fills=",
        [
            ("cb_deopt", Max(3)),
            ("cb_xstep", Min(390)),
            // One frame is materialised later in the first activation (the temporary area has
            // room from the start, #730), so the interpreter finishes a longer stretch of it.
            ("ops", Max(1460)),
            ("llvm_native_frames", Min(3)),
        ]
    );
    // A process without a wait runs its jumps back in compiled code, leaving every 2^20th (#583).
    llvm_case!(
        llvm_nowait,
        "jumps=",
        [("cb_step", Max(4)), ("cb_signal", Max(2))]
    );
    // Constants found once stay for the run (#679).
    llvm_case!(llvm_hostless, "acc=", [("cb_host", Max(2))]);
    // A deallocation drops only the freed object from compiled code's table (#520): 600 `cb_fast`
    // before (every `deallocate` made both `Deref`s of the table after it ask the host again), 201 now.
    llvm_case!(llvm_heap_free, "acc=", [("cb_fast", Max(207))]);
    // Functions and procedures that read signals run on native frames (#519): 900 calls, no step.
    llvm_case!(
        llvm_sigfun,
        "acc=",
        [
            ("cb_step", Max(4)),
            ("cb_signal", Max(4)),
            ("llvm_native_frames", Min(2))
        ]
    );
    // Procedures that drive signals and a function that reads a vector run on native frames
    // (#730): 900 calls, no step, no frame materialised.
    llvm_case!(
        llvm_sigdrive,
        "acc=",
        [
            ("cb_step", Max(2)),
            ("cb_signal", Max(302)),
            ("cb_deopt", Max(0)),
            ("llvm_native_frames", Min(3))
        ]
    );
    // `uniform` is computed in compiled code (#519): 400 calls, no callback for them.
    llvm_case!(
        llvm_uniform,
        "acc=",
        [("cb_native", Max(4)), ("cb_host", Max(2))]
    );
    // The memo of pure calls (#701) answers the compiled calls too: the same hits as the interpreter.
    #[test]
    fn llvm_pure_memo() {
        check(
            "pure_memo",
            include_str!("perf/pure_memo.vhd"),
            "q=9",
            "llvm",
            &[],
            &[
                ("pmemo_hits", Min(420)),
                ("pmemo_runs", Max(14)),
                ("cb_memo", Max(470)),
            ],
        );
    }
    llvm_case!(
        llvm_ieee,
        "acc=",
        [
            ("cb_native", Max(5)),
            ("cb_host", Max(3)),
            ("llvm_native_frames", Min(1)),
        ]
    );
    // A field of a record whose layout depends on a generic, read on a native frame: one call of
    // the layout memo per field instead of a step of the `LayoutOf` (#679).
    llvm_case!(
        llvm_layout_field,
        "acc=38682",
        [
            ("cb_xstep", Max(0)),
            ("cb_step", Max(5)),
            ("cb_layout", Max(1648))
        ]
    );
    llvm_case!(
        llvm_eloop,
        "acc=",
        [
            // The fills of `ones`; the row copies (one plane block each side) are inline: a
            // copy that takes the runtime call again raises it (180 before #757 round 2).
            ("cb_eloop", Min(20)),
            ("cb_eloop", Max(20)),
            ("cb_step", Max(6)),
            ("cb_deopt", Max(0)),
            // The planned loops find their vectors and descriptors inline (fresh frames, the
            // caller's temporaries, held parameter regions, the object table): no lookup call.
            ("cb_host", Max(0)),
            // The descriptor parameter's region is found in the object table at the entry.
            ("cb_region", Max(1)),
            ("llvm_native_frames", Min(2)),
        ]
    );
    llvm_case!(
        llvm_drive_batch,
        "q=",
        [
            ("cb_drive", Max(158)),
            ("llvm_interpreted_insts", Max(32)),
            // Kept drives of unchanged elements make no kernel entry, as in the interpreter.
            ("direct_codes", Max(107)),
        ]
    );
    llvm_case!(
        llvm_clocked,
        "q=",
        [
            ("cb_signal", Max(4)),
            ("cb_step", Max(26)),
            ("cb_native", Max(4)),
            ("cb_drive", Max(998)),
            ("cb_wait", Max(19)),
            ("cb_wait_same", Max(1018)),
            ("llvm_inline_insts", Min(600)),
            ("llvm_interpreted_insts", Max(48)),
            ("llvm_native_frames", Min(1)),
        ]
    );

    /// `edge_cost_parity` compiled (#772): the `wait until` form ends its wait on the kernel as
    /// the sensitivity-list forms do (`static_wait_edge`), so all four make the same callbacks.
    #[test]
    fn llvm_edge_cost_parity() {
        let forms = [
            "process (clk) begin if rising_edge(clk) then q(i) <= not q(i); end if; end process;",
            "process (clk) begin if clk'event and clk = '1' then q(i) <= not q(i); end if; end process;",
            "process (clk) begin if not clk'stable and clk = '1' then q(i) <= not q(i); end if; end process;",
            "process begin wait until rising_edge(clk); q(i) <= not q(i); end process;",
        ];
        let template = include_str!("perf/edge_cost.vhd");
        let mut cbs = Vec::new();
        for (k, form) in forms.iter().enumerate() {
            let d = tempdir(&format!("llvm_edge_cost_{k}"));
            let r = run_design(
                &d,
                "edge_cost",
                &template.replace(forms[0], form),
                "llvm",
                &[],
            );
            assert_eq!(r.code, 0, "{form}: {}", r.all());
            assert!(r.all().contains("ones=8"), "{form}: {}", r.all());
            check_bounds(
                &format!("llvm_edge_cost/{k}"),
                &r,
                &[("cb_wait_same", Max(1050)), ("cb_signal", Max(16))],
            );
            let sum: u64 = (r.counters.iter())
                .filter(|(c, _)| c.starts_with("cb_"))
                .map(|(_, &n)| n)
                .sum();
            cbs.push(sum);
        }
        // The wait form waits in full at its first activation (no edge resumed it yet).
        assert!(
            cbs.iter().all(|&n| n <= cbs[0] + 16 && n + 16 >= cbs[0]),
            "callbacks per form: {cbs:?}"
        );
    }
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

    clift_case!(
        clocked,
        "llvm_clocked",
        "q=",
        [
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
            ("cb_signal", Max(60)),
            ("cb_step", Max(9)),
            ("cb_native", Max(56)),
            ("cb_drive", Max(263)),
            ("cb_drive", Min(230)),
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
    // The memo of pure calls (#701) answers the compiled calls of this tier too.
    clift_case!(
        pure_memo,
        "pure_memo",
        "q=9",
        [
            ("pmemo_hits", Min(420)),
            ("pmemo_runs", Max(14)),
            ("cb_memo", Max(470)),
            ("cb_memo", Min(430)),
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
    // Element loops of native frames through `yavs_rt_eloop` (#757).
    clift_case!(
        eloop,
        "llvm_eloop",
        "acc=",
        [
            ("cb_eloop", Min(175)),
            ("cb_step", Max(6)),
            ("cb_deopt", Max(0))
        ]
    );
    clift_case!(
        reg_planes,
        "reg_planes",
        "c=100",
        [
            ("cb_signal", Max(5)),
            ("cb_step", Max(10)),
            ("cb_drive", Max(416)),
            ("cb_drive", Min(380)),
            ("cb_native", Max(4)),
        ]
    );
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
