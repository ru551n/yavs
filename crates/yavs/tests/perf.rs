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
            // Counted in debug builds and with `op-census`; a plain release build has none (`ops`, `cb_*`).
            assert!(
                name == "ops" || name.starts_with("cb_"),
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
        ("ops", Max(3258)),
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
        ("ops", Max(5245)),
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
        ("ops", Max(6802)),
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
        ("ops", Max(7734)),
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
        ("ops", Max(13445)),
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
        ("ops", Max(8428)),
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
        ("ops", Max(506)),
    ]
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
        ("ops", Max(10455)),
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
        ("ops", Max(10427)),
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
        ("ops", Max(13659)),
    ]
);
case!(
    reduce_idiom,
    "n1=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(2098)),]
);
case!(elem_loops, "acc=", &[], [("ops", Max(35600))]);
case!(
    licm_loop,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(26442)),]
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
        ("ops", Max(6934)),
    ]
);
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
        ("ops", Max(7977)),
    ]
);
case!(
    inline_calls,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(3320)),]
);
case!(
    static_chain,
    "acc=",
    &[],
    [("process_runs", Max(2)), ("ops", Max(6520)),]
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
        ("ops", Max(4579)),
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
        ("ops", Max(1004)),
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
        ("ops", Max(1822)),
    ]
);

case!(
    idle_regs,
    "q=42,28,70,0,70,44 b='1''1'",
    &[],
    [
        ("process_runs", Max(3100)),
        ("idle_skips", Min(4050)),
        ("ops", Max(34700)),
    ]
);

/// The idle skip changes nothing a run shows: the same output (values, times and deltas of the
/// reports) with it off, on and checked (a skipped activation run and compared), and it skips.
#[test]
fn idle_skip_changes_nothing() {
    let d = tempdir("idle_equal");
    let src = include_str!("perf/idle_regs.vhd");
    let run = |mode: &str| {
        run_design_env(
            &d,
            "idle_regs",
            src,
            "interp",
            &[],
            &[("YAVS_IDLE_SKIP", mode)],
        )
    };
    let (off, on, check) = (run("off"), run("on"), run("check"));
    assert_eq!(
        (off.code, on.code, check.code),
        (0, 0, 0),
        "{}",
        check.all()
    );
    assert_eq!(off.stdout, on.stdout);
    assert_eq!(off.stdout, check.stdout);
    assert_eq!(off.counters["idle_skips"], 0);
    assert!(
        on.counters["idle_skips"] > 4000,
        "the skip did not fire: {:?}",
        on.counters
    );
    assert_eq!(on.counters["idle_skips"], check.counters["idle_skips"]);
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
            ("cb_step", Max(9)),
            ("cb_native", Max(56)),
            ("cb_drive", Max(263)),
            ("cb_troot", Max(53)),
            ("cb_endstmt", Max(53)),
            ("cb_lcopy", Max(106)),
            ("cb_fast", Max(1)),
            ("llvm_inline_insts", Min(612)),
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
        llvm_clocked,
        "q=",
        [
            ("cb_signal", Max(4)),
            ("cb_step", Max(6)),
            ("cb_native", Max(55)),
            ("cb_drive", Max(998)),
            ("cb_wait", Max(911)),
            ("cb_wait_same", Max(106)),
            ("llvm_inline_insts", Min(624)),
            ("llvm_interpreted_insts", Max(48)),
            ("llvm_native_frames", Min(1)),
        ]
    );
}
