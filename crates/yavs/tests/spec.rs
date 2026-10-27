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

/// Under `--tier=auto` with every function compiled when the interpreter enters it
/// (`--tier-heat=1,0,0`) a process switches to its copy after the default number of activations
/// although its shared code is compiled (`Tier::compiles_copies`, #679), and the report is the
/// interpreter's.
#[cfg(feature = "llvm")]
#[test]
fn compiled_processes_switch_to_copies_under_auto() {
    let d = tempdir("spec_auto_default");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/spec_copies.vhd"),
    )
    .expect("design")
    .replace("1500 ns", "40 us");
    std::fs::write(d.join("spec_copies.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(&d, &[], &["analyze", "--work", &work, "spec_copies.vhd"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    let go = |tier_args: &[&str]| {
        let mut args = vec!["run", "--work", &work, "--no-cache", "--threads", "1"];
        args.extend_from_slice(tier_args);
        args.push("spec_copies");
        yavs(&d, &[], &args)
    };
    let interp = go(&["--tier=interp"]);
    assert_eq!(interp.code, 0, "{}", interp.stderr);
    let heat = go(&["--tier=auto", "--tier-heat=1,0,0"]);
    assert_eq!(heat.code, 0, "{}", heat.stderr);
    assert_eq!(heat.stdout, interp.stdout);
    assert!(heat.counters["spec_switched"] >= 6);
}

/// Compiled code is kept in the compilation cache under a key made of the generated code. A copy
/// is named by what it is for, not by its function id (which follows the order in which the
/// processes switched, and that differs between thread counts), so a second run that switches in
/// another order links every module from the cache instead of compiling any (#661).
#[cfg(feature = "llvm")]
#[test]
fn compiled_copies_are_cached_by_content() {
    let d = tempdir("spec_objcache");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/spec_copies.vhd"),
    )
    .expect("design");
    std::fs::write(d.join("spec_copies.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(&d, &[], &["analyze", "--work", &work, "spec_copies.vhd"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    // (A shuffled run, its seed chosen at random, starts the processes of a cycle in another order,
    // so they switch in another.)
    let go = |threads: &str| {
        yavs(
            &d,
            &[("YAVS_SPEC_AFTER", "2"), ("YAVS_LOG", "debug")],
            &[
                "run",
                "--work",
                &work,
                "--tier=auto",
                "--tier-heat=1,0,0",
                "--threads",
                threads,
                "--shuffle",
                "spec_copies",
            ],
        )
    };
    let count = |r: &Run, what: &str| {
        r.stderr
            .lines()
            .filter(|l| l.starts_with(&format!("debug: cache: obj {what} ")))
            .count()
    };
    // (A hit of a pre-key links the object without generating the code, #785.)
    let hits = |r: &Run| count(r, "hit") + count(r, "prehit");
    let first = go("1");
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert!(first.counters["spec_switched"] >= 6);
    assert!(count(&first, "stored") > 0, "{}", first.stderr);
    // The compile thread's queue ends where the simulation does, and a run that links from the
    // cache gets further down it than the one that compiled (#672: the second run stored a module
    // the first never reached). Repeat until a run stores nothing: the cache then holds every
    // module a run can reach, and from here the order and thread count must not matter.
    let settled = ["4", "1", "4", "1", "4"].iter().any(|t| {
        let r = go(t);
        assert_eq!(r.code, 0, "{}", r.stderr);
        count(&r, "stored") == 0
    });
    assert!(settled, "the cache never held every module");
    for threads in ["4", "1", "4", "4"] {
        let again = go(threads);
        assert_eq!(again.code, 0, "{}", again.stderr);
        assert_eq!(
            count(&again, "stored"),
            0,
            "--threads {threads}: {}",
            again.stderr
        );
        assert!(hits(&again) >= count(&first, "stored"));
    }
}

/// A warm run links the compiled code a pre-key names without generating it (#785), whatever
/// order the processes switched to their copies in; under `YAVS_LLVM_PREKEY_CHECK` (CI) it
/// generates the code anyway and must find the object the pre-key named (a difference aborts).
#[cfg(feature = "llvm")]
#[test]
fn a_warm_run_links_by_the_pre_key_and_the_check_agrees() {
    let d = tempdir("spec_prekey");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/perf/spec_copies.vhd"),
    )
    .expect("design");
    std::fs::write(d.join("spec_copies.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(&d, &[], &["analyze", "--work", &work, "spec_copies.vhd"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    let go = |threads: &str, check: &str| {
        let r = yavs(
            &d,
            &[
                ("YAVS_SPEC_AFTER", "2"),
                ("YAVS_LOG", "debug"),
                ("YAVS_LLVM_PREKEY_CHECK", check),
            ],
            &[
                "run",
                "--work",
                &work,
                "--tier=auto",
                "--tier-heat=1,0,0",
                "--threads",
                threads,
                "--shuffle",
                "spec_copies",
            ],
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        let n = |what: &str| {
            r.stderr
                .lines()
                .filter(|l| l.starts_with(&format!("debug: cache: obj {what} ")))
                .count()
        };
        (r.stdout.clone(), n("prehit"), n("pre checked"))
    };
    // The first run has no stored program yet (the closure's entries come with its second run),
    // so no pre-key; the second writes them.
    let cold = go("1", "");
    go("4", "");
    let warm = go("4", "");
    assert!(warm.1 > 0, "nothing linked by a pre-key");
    assert_eq!(warm.2, 0);
    for threads in ["1", "4"] {
        let checked = go(threads, "1");
        assert_eq!(checked.1, 0, "the check generates every function");
        assert!(checked.2 >= warm.1, "{checked:?} against {warm:?}");
        assert_eq!(checked.0, cold.0);
    }
    assert_eq!(warm.0, cold.0);
}

/// A copy knows the bounds of a signal array in the instance data and splits the position of an
/// index of an ascending range into a check and a subtraction (`pos_to_check`): an index outside
/// the bounds fails with the shared code's text, for ascending and descending ranges, and in
/// range it addresses the same elements.
#[test]
fn index_failures_in_a_copy_are_those_of_the_shared_code() {
    for (tag, rng) in [("to", "0 to W - 1"), ("downto", "W - 1 downto 0")] {
        let design = format!("idx_{tag}");
        let src = format!(
            "entity {design} is generic (W : positive := 3); end;
architecture a of {design} is
  type row_t is array ({rng}) of bit_vector(3 downto 0);
  type mat_t is array ({rng}) of row_t;
  signal m : mat_t := (others => (others => x\"0\"));
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 400 ns;
  process (clk)
    variable i : integer := 0;
  begin
    if clk = '1' then
      i := i + 1;
      m(i mod 4)(i mod 3) <= x\"5\";
      m(0)((i * 2) mod 4) <= m(0)(i mod 3) xor x\"1\";
    end if;
  end process;
end;
"
        );
        let d = tempdir(&design);
        std::fs::write(d.join(format!("{design}.vhd")), src).expect("write");
        let work = format!("work:{}", d.join("lib").display());
        let a = yavs(
            &d,
            &[],
            &["analyze", "--work", &work, &format!("{design}.vhd")],
        );
        assert_eq!(a.code, 0, "analyze {design}: {}", a.stderr);
        let shared = run(&d, &design, "1", &[("YAVS_NO_SPEC", "1")]);
        let copy = run(&d, &design, "1", &[("YAVS_SPEC_AFTER", "2")]);
        assert!(
            shared.stderr.contains("outside of range"),
            "{design}: {}",
            shared.stderr
        );
        assert_eq!(shared.code, copy.code, "{design}: exit status");
        assert_eq!(shared.stdout, copy.stdout, "{design}: stdout");
        assert_eq!(shared.stderr, copy.stderr, "{design}: stderr");
        assert!(copy.counters["spec_switched"] > 0, "{design}: no copy");
        // The copy has the split checks the shared code lacks (an ascending range only: the check
        // message of an index says `to`).
        let checks = |env: &[(&str, &str)]| {
            let mut e = env.to_vec();
            e.push(("YAVS_DUMPIR", "1"));
            run(&d, &design, "1", &e)
                .stderr
                .matches("kind: Index")
                .count()
        };
        assert_eq!(
            checks(&[("YAVS_SPEC_AFTER", "2")]) > checks(&[("YAVS_NO_SPEC", "1")]),
            tag == "to",
            "{design}: checks in the copy"
        );
    }
}
