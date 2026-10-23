//! Specialised copies of processes (`docs/design/specialisation.md`): a process that runs its copy
//! produces exactly what the shared code produces. Every design of `perf/` is run with copies
//! off (`YAVS_NO_SPEC`) and with every process switching at its second activation
//! (`YAVS_SPEC_AFTER=2`), on one and on four threads, and the output must be byte for byte the
//! same; the counters say that processes did switch.

mod common;
use common::tempdir;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    counters: BTreeMap<String, u64>,
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
    let all = String::from_utf8_lossy(&o.stderr).into_owned();
    let counters = all
        .lines()
        .filter_map(|l| l.strip_prefix("counters: ")?.split_once('='))
        .map(|(k, v)| (k.to_string(), v.parse().expect("counter value")))
        .collect();
    // What a run says apart from the counters (those differ by design).
    let stderr = all
        .lines()
        .filter(|l| !l.starts_with("counters: "))
        .map(|l| format!("{l}\n"))
        .collect();
    Run {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr,
        counters,
    }
}

fn run(dir: &Path, design: &str, threads: &str, env: &[(&str, &str)]) -> Run {
    let work = format!("work:{}", dir.join("lib").display());
    yavs(
        dir,
        env,
        &[
            "run",
            "--work",
            &work,
            "--tier=interp",
            "--no-cache",
            "--threads",
            threads,
            design,
        ],
    )
}

/// The designs of `perf/` that are plain testbenches (the top entity is the file's name).
fn designs() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .expect("perf designs")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "vhd").then(|| {
                (
                    p.file_stem().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).expect("read"),
                )
            })
        })
        // These need arguments or companions of their own (`perf.rs`).
        .filter(|(n, _)| {
            !matches!(
                n.as_str(),
                "ecall_pkg" | "ecall_tb" | "unit_pkg" | "unit_mid" | "unit_tb"
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn copies_change_nothing() {
    let mut switched = 0;
    let mut copies = 0;
    for (design, src) in designs() {
        let d = tempdir(&design);
        std::fs::write(d.join(format!("{design}.vhd")), &src).expect("write");
        let work = format!("work:{}", d.join("lib").display());
        let a = yavs(
            &d,
            &[],
            &["analyze", "--work", &work, &format!("{design}.vhd")],
        );
        assert_eq!(a.code, 0, "analyze {design}: {}", a.stderr);
        for threads in ["1", "4"] {
            let shared = run(&d, &design, threads, &[("YAVS_NO_SPEC", "1")]);
            let copy = run(&d, &design, threads, &[("YAVS_SPEC_AFTER", "2")]);
            assert_eq!(
                shared.code, copy.code,
                "{design} --threads {threads}: exit status"
            );
            assert_eq!(
                shared.stdout, copy.stdout,
                "{design} --threads {threads}: stdout"
            );
            assert_eq!(
                shared.stderr, copy.stderr,
                "{design} --threads {threads}: stderr"
            );
            assert_eq!(shared.counters.get("spec_switched"), Some(&0), "{design}");
            switched += copy.counters["spec_switched"];
            copies += copy.counters["spec_copies"];
        }
    }
    assert!(
        switched > 0 && copies > 0,
        "no process switched to a copy in any design"
    );
}

/// The copies a run made are stored with the design (stage cache, kind `spec`) and the next run
/// reads them instead of optimising again: same output, `spec_loaded` instead of `spec_fresh`.
#[test]
fn copies_are_stored_between_runs() {
    let d = tempdir("spec_cache");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/spec_copies.vhd"),
    )
    .expect("design");
    std::fs::write(d.join("spec_copies.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(&d, &[], &["analyze", "--work", &work, "spec_copies.vhd"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    let go = || {
        yavs(
            &d,
            &[("YAVS_SPEC_AFTER", "2")],
            &[
                "run",
                "--work",
                &work,
                "--tier=interp",
                "--threads",
                "1",
                "spec_copies",
            ],
        )
    };
    // The first runs of a closure only leave notes and the analysis image; the lowering and
    // with it the copies are stored once the design is cached, then read.
    let runs: Vec<Run> = (0..5).map(|_| go()).collect();
    let last = runs.last().unwrap();
    for r in &runs {
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, last.stdout);
        assert_eq!(r.counters["spec_copies"], 3);
    }
    assert_eq!(last.counters["spec_fresh"], 0, "{:?}", last.counters);
    assert_eq!(last.counters["spec_loaded"], 3, "{:?}", last.counters);
    let kinds: Vec<String> = std::fs::read_dir(d.join("lib").join("yavs-cache"))
        .expect("cache")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(kinds.iter().any(|n| n.starts_with("spec-")), "{kinds:?}");
}

/// Under `--tier=auto` a copy is a function like any other: with every function compiled when
/// the interpreter enters it (`--tier-heat=1,0,0`) the processes that switched run compiled
/// copies, and the report is the one of the shared code.
#[cfg(feature = "llvm")]
#[test]
fn copies_compile_under_auto() {
    let d = tempdir("spec_auto");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/spec_copies.vhd"),
    )
    .expect("design");
    std::fs::write(d.join("spec_copies.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(&d, &[], &["analyze", "--work", &work, "spec_copies.vhd"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    let go = |env: &[(&str, &str)], heat: &[&str]| {
        let mut args = vec![
            "run",
            "--work",
            &work,
            "--tier=auto",
            "--no-cache",
            "--threads",
            "1",
        ];
        args.extend_from_slice(heat);
        args.push("spec_copies");
        yavs(&d, env, &args)
    };
    let shared = go(&[("YAVS_NO_SPEC", "1")], &[]);
    assert_eq!(shared.code, 0, "{}", shared.stderr);
    for heat in [&[][..], &["--tier-heat=1,0,0"][..]] {
        let copies = go(&[("YAVS_SPEC_AFTER", "2")], heat);
        assert_eq!(copies.code, 0, "{heat:?}: {}", copies.stderr);
        assert_eq!(copies.stdout, shared.stdout, "{heat:?}");
        assert_eq!(copies.stderr, shared.stderr, "{heat:?}");
        assert!(copies.counters["spec_switched"] >= 6, "{heat:?}");
    }
}
