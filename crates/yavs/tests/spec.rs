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
        // The value follows the last `=`: `ieee_general` keys spell operators such as "=".
        .filter_map(|l| l.strip_prefix("counters: ")?.rsplit_once('='))
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

/// The tiers that tier up under heat modes: the Cranelift tier, which runs as `clift-auto`.
#[cfg(feature = "clift")]
fn auto_tiers() -> Vec<&'static str> {
    vec!["clift-auto"]
}

/// Under tier-up a copy is a function like any other: with every function compiled when the
/// interpreter enters it (`--tier-heat=1,0,0`, which waits for each compile, so nothing here
/// depends on the sampler) the processes that switched run compiled copies, and the report is the
/// one of the shared code.
#[cfg(feature = "clift")]
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
    for tier in auto_tiers() {
        let tier = format!("--tier={tier}");
        let go = |env: &[(&str, &str)], heat: &[&str]| {
            let mut args = vec![
                "run",
                "--work",
                &work,
                &tier,
                "--no-cache",
                "--threads",
                "1",
            ];
            args.extend_from_slice(heat);
            args.push("spec_copies");
            yavs(&d, env, &args)
        };
        let shared = go(&[("YAVS_NO_SPEC", "1")], &[]);
        assert_eq!(shared.code, 0, "{tier}: {}", shared.stderr);
        for heat in [&[][..], &["--tier-heat=1,0,0"][..]] {
            let copies = go(&[("YAVS_SPEC_AFTER", "2")], heat);
            assert_eq!(copies.code, 0, "{tier} {heat:?}: {}", copies.stderr);
            assert_eq!(copies.stdout, shared.stdout, "{tier} {heat:?}");
            assert_eq!(copies.stderr, shared.stderr, "{tier} {heat:?}");
            assert!(copies.counters["spec_switched"] >= 6, "{tier} {heat:?}");
        }
        // The compile trace names the copies compiled (the sampler's trace format is shared by
        // the tiers; only Cranelift's is checked here). All three copies are asked for on
        // entry and the run waits for each.
        if tier == "--tier=clift-auto" {
            let traced = go(
                &[("YAVS_SPEC_AFTER", "2"), ("YAVS_AUTO_TRACE", "1")],
                &["--tier-heat=1,0,0"],
            );
            let n = traced.stderr.matches(", copy of f").count();
            assert!(n >= 3, "{n} compiled copies:\n{}", traced.stderr);
        }
    }
}

/// Under tier-up with every function compiled when the interpreter enters it
/// (`--tier-heat=1,0,0`) a process switches to its copy after the default number of activations
/// although its shared code is compiled (`Tier::compiles_copies`, #679), and the report is the
/// interpreter's.
#[cfg(feature = "clift")]
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
    for tier in auto_tiers() {
        let tier = format!("--tier={tier}");
        let heat = go(&[&tier, "--tier-heat=1,0,0"]);
        assert_eq!(heat.code, 0, "{tier}: {}", heat.stderr);
        assert_eq!(heat.stdout, interp.stdout, "{tier}");
        assert!(heat.counters["spec_switched"] >= 6, "{tier}");
    }
}

/// A copy knows the bounds of a signal array in the instance data and splits the position of an
/// index into a check and a subtraction (`pos_to_check`): an index outside the bounds fails with
/// the shared code's text, for ascending and descending ranges, and in range it addresses the same
/// elements.
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
        // The same failure from compiled code of the copy.
        #[cfg(feature = "clift")]
        {
            let c = yavs(
                &d,
                &[("YAVS_SPEC_AFTER", "2")],
                &[
                    "run",
                    "--work",
                    &work,
                    "--tier=auto",
                    "--tier-heat=1,0,0",
                    "--no-cache",
                    "--threads=1",
                    &design,
                ],
            );
            assert_eq!(shared.code, c.code, "{design}: compiled exit status");
            assert_eq!(shared.stdout, c.stdout, "{design}: compiled stdout");
            assert_eq!(shared.stderr, c.stderr, "{design}: compiled stderr");
        }
        // The copy has the split checks the shared code lacks.
        let checks = |env: &[(&str, &str)]| {
            let mut e = env.to_vec();
            e.push(("YAVS_DUMPIR", "1"));
            run(&d, &design, "1", &e)
                .stderr
                .matches("kind: Index")
                .count()
        };
        assert!(
            checks(&[("YAVS_SPEC_AFTER", "2")]) > checks(&[("YAVS_NO_SPEC", "1")]),
            "{design}: checks in the copy"
        );
    }
}

/// A procedure nested in a process is called on a native frame from the process's copy too: the
/// copy's frame is the static link, and a variable passed by reference and one reached through the
/// link are the same ones as in the shared code.
#[cfg(feature = "clift")]
#[test]
fn a_copy_calls_what_is_nested_in_its_process_on_a_native_frame() {
    let design = "copy_nested";
    let src = "entity copy_nested is generic (W : positive := 3); end;
architecture a of copy_nested is
  signal clk : bit := '0';
  signal s : integer := 0;
begin
  clk <= not clk after 5 ns when now < 2 us;
  process (clk)
    variable total : integer := 0;
    variable mine : integer := 0;
    procedure add(variable x : inout integer; n : integer) is
    begin
      for i in 1 to n loop
        x := x + W;
        total := total + i;
      end loop;
    end procedure;
  begin
    if clk = '1' then
      if W > 100 then report \"big\"; end if;
      add(mine, 40);
      add(total, 3);
      s <= mine + total;
      if now > 1950 ns then
        report integer'image(mine) & \" \" & integer'image(total);
      end if;
    end if;
  end process;
end;
";
    let d = tempdir(design);
    std::fs::write(d.join(format!("{design}.vhd")), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--work", &work, &format!("{design}.vhd")],
    );
    assert_eq!(a.code, 0, "analyze: {}", a.stderr);
    let want = run(&d, design, "1", &[("YAVS_NO_SPEC", "1")]);
    assert_eq!(want.code, 0, "{}", want.stderr);
    let c = yavs(
        &d,
        &[("YAVS_SPEC_AFTER", "2")],
        &[
            "run",
            "--work",
            &work,
            "--tier=auto",
            "--tier-heat=1,0,0",
            "--no-cache",
            "--threads=1",
            design,
        ],
    );
    assert_eq!(c.code, want.code, "{}", c.stderr);
    assert_eq!(c.stderr, want.stderr, "stderr");
    assert!(c.counters["spec_switched"] > 0, "no copy");
    // 200 activations of two calls each (a release test build has no such counter).
    if let Some(&steps) = c.counters.get("cb_step") {
        assert!(steps < 100, "the calls were stepped: {steps}");
    }
}
