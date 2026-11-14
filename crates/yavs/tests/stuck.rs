//! Diagnostics for stuck and hung simulations: delta cycle loops (E3002), a process that can
//! never suspend (W3102), the hang report (W3101, `--hang-report` and SIGUSR1) and the seed of
//! a shuffled run. Snapshots are `tests/cli/NAME.stderr`; `YAVS_BLESS=1` rewrites them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli")
}

fn tmp(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("yavs-stuck-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("tmp");
    p
}

fn yavs(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .current_dir(dir())
        .output()
        .expect("run yavs")
}

/// Analyze `tests/cli/NAME.vhd` (2008) and run its entity with `extra`; returns the output.
fn run(name: &str, extra: &[&str]) -> Output {
    run_env(name, extra, &[])
}

/// `run` with environment variables set for the run.
fn run_env(name: &str, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let t = tmp(name);
    let lib = format!("--work=work:{}/lib", t.display());
    let file = format!("{name}.vhd");
    assert!(
        yavs(&["analyze", "--std=2008", &lib, &file])
            .status
            .success()
    );
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args([&["run", &lib][..], extra, &[name]].concat())
        .envs(env.iter().copied())
        .current_dir(dir())
        .output()
        .expect("run yavs");
    let _ = std::fs::remove_dir_all(&t);
    o
}

fn clean(o: &Output) -> String {
    let canon = dir().canonicalize().expect("dir");
    let canon = canon.to_str().expect("utf-8");
    let canon = canon.strip_prefix(r"\\?\").unwrap_or(canon);
    String::from_utf8(o.stderr.clone())
        .expect("utf-8")
        .replace(canon, "$DIR")
        .replace(r"$DIR\", "$DIR/")
}

/// Run and compare stderr with `snapshot.stderr`.
fn snapshot(name: &str, extra: &[&str], snapshot: &str, code: i32) -> String {
    snapshot_env(name, extra, &[], snapshot, code)
}

/// `snapshot` with environment variables set for the run.
fn snapshot_env(
    name: &str,
    extra: &[&str],
    env: &[(&str, &str)],
    snapshot: &str,
    code: i32,
) -> String {
    let o = run_env(name, extra, env);
    assert_eq!(o.status.code(), Some(code), "{}", clean(&o));
    let err = clean(&o);
    let path = dir().join(format!("{snapshot}.stderr"));
    if std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1") {
        std::fs::write(&path, &err).expect("write");
    } else {
        assert_eq!(err, std::fs::read_to_string(&path).unwrap_or_default());
    }
    err
}

#[test]
fn delta_loop_between_two_processes() {
    let e = snapshot("stuck_pingpong", &[], "stuck_pingpong", 1);
    assert!(e.contains("wake each other through"), "{e}");
    // The cause does not depend on the thread count.
    let o = run("stuck_pingpong", &["--threads=4"]);
    assert_eq!(clean(&o), e);
}

#[test]
fn delta_loop_through_a_concurrent_assignment() {
    snapshot("stuck_comb", &[], "stuck_comb", 1);
}

#[test]
fn delta_loop_of_a_process_waiting_on_its_own_signal() {
    snapshot("stuck_self", &["--max-deltas=50"], "stuck_self", 1);
}

#[test]
fn delta_loop_of_wait_for_zero() {
    snapshot("stuck_zero", &[], "stuck_zero", 1);
}

#[test]
fn max_deltas_is_validated() {
    let o = run("stuck_comb", &["--max-deltas=0"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn process_without_wait_is_warned_about() {
    snapshot("stuck_nowait", &[], "stuck_nowait", 0);
    // Not an error: -Wno hides it.
    let o = run("stuck_nowait", &["-Wno-W3102"]);
    assert!(o.stderr.is_empty(), "{}", clean(&o));
}

fn tiers() -> Vec<&'static str> {
    let mut tiers = vec!["--tier=interp"];
    if cfg!(feature = "clift") {
        tiers.extend(["--tier=clift", "--tier=clift-auto"]);
    }
    tiers
}

/// The activation of a process that never suspends is bounded in every tier (#583), at the same
/// count: `YAVS_SPIN_LIMIT` lowers the bound (not a multiple of the compiled code's check
/// interval, so the exact count is tested), and a run of `last` jumps back ends by itself or with
/// E3009 on either side of it.
#[test]
fn a_process_that_never_suspends_ends_with_an_error() {
    let env = [("YAVS_SPIN_LIMIT", "2000000")];
    for tier in tiers() {
        let o = run_env("stuck_spin", &[tier], &env);
        assert_eq!(o.status.code(), Some(1), "{tier}: {}", clean(&o));
        let e = String::from_utf8_lossy(&o.stderr);
        assert!(e.contains("E3009"), "{tier}: {e}");
        // Just inside the bound: the run finishes. Just outside: E3009.
        for (last, ok) in [("1999900", true), ("2000100", false)] {
            let o = run_env("stuck_count", &[tier, &format!("-glast={last}")], &env);
            let all = String::from_utf8_lossy(&o.stdout).to_string() + &clean(&o);
            assert_eq!(
                o.status.code(),
                Some(i32::from(!ok)),
                "{tier} {last}: {all}"
            );
            assert_eq!(all.contains("counted"), ok, "{tier} {last}: {all}");
            assert_eq!(all.contains("E3009"), !ok, "{tier} {last}: {all}");
        }
    }
}

/// A loop without a wait whose jump back is reached from an `if` without an else is bounded in
/// every tier, at the same count (#1087): the predecoder threaded the false edge past the jump,
/// so nothing counted it, and compiled code that left at it was entered again at once.
#[test]
fn a_loop_whose_jump_back_is_threaded_ends_with_an_error() {
    let env = [("YAVS_SPIN_LIMIT", "2000000")];
    for tier in tiers() {
        // The count is never reached: the loop runs until it is diagnosed.
        let o = run_env("stuck_loop", &[tier], &env);
        let e = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{tier}: {e}");
        assert!(e.contains("E3009"), "{tier}: {e}");
        for (last, ok) in [("1999900", true), ("2000100", false)] {
            let o = run_env("stuck_loop", &[tier, &format!("-glast={last}")], &env);
            let all = String::from_utf8_lossy(&o.stdout).to_string() + &clean(&o);
            assert_eq!(
                o.status.code(),
                Some(i32::from(!ok)),
                "{tier} {last}: {all}"
            );
            assert_eq!(all.contains("reached 1"), ok, "{tier} {last}: {all}");
            assert_eq!(all.contains("E3009"), !ok, "{tier} {last}: {all}");
        }
    }
}

/// A long loop without a wait that ends with `finish` runs to its end in every tier, past
/// several trims and checks of the compiled code (#583).
#[test]
fn a_long_loop_without_a_wait_runs_to_its_finish() {
    for tier in tiers() {
        let o = run("stuck_count", &[tier, "-glast=3000000"]);
        let out = String::from_utf8_lossy(&o.stdout);
        assert_eq!(o.status.code(), Some(0), "{tier}: {out}{}", clean(&o));
        assert!(out.contains("counted 3000000"), "{tier}: {out}");
    }
}

#[test]
fn hang_report_at_stop_time() {
    let e = snapshot(
        "stuck_forever",
        &["--stop-time=1us", "--hang-report"],
        "stuck_forever",
        0,
    );
    assert!(e.contains("W3101") && e.contains("no driver"), "{e}");
    // Off unless asked for.
    let o = run("stuck_forever", &["--stop-time=1us"]);
    assert!(o.stderr.is_empty(), "{}", clean(&o));
    // Identical at any thread count.
    let o = run(
        "stuck_forever",
        &["--stop-time=1us", "--hang-report", "--threads=4"],
    );
    assert_eq!(clean(&o), e);
}

#[test]
fn hang_report_when_a_clock_stops() {
    let e = snapshot("stuck_clock", &["--hang-report"], "stuck_clock", 0);
    assert!(e.contains("behaved like a clock"), "{e}");
}

#[test]
fn no_hang_report_after_a_clean_end() {
    let o = run("stuck_nowait", &["--hang-report", "-Wno-W3102"]);
    assert!(o.stderr.is_empty(), "{}", clean(&o));
}

#[test]
fn stuck_diagnostics_in_json() {
    let o = run(
        "stuck_forever",
        &["--stop-time=1us", "--hang-report", "--message-format=json"],
    );
    let line = String::from_utf8(o.stderr).expect("utf-8");
    let v: serde_json::Value = serde_json::from_str(line.lines().next().expect("line")).unwrap();
    assert_eq!(v["code"], "W3101");
    assert_eq!(v["level"], "warning");
    let kids = v["children"].to_string();
    assert!(kids.contains("no driver"), "{kids}");
    let o = run("stuck_pingpong", &["--message-format=json"]);
    let line = String::from_utf8(o.stderr).expect("utf-8");
    let v: serde_json::Value = serde_json::from_str(line.lines().next().expect("line")).unwrap();
    assert_eq!(v["code"], "E3002");
    let kids = v["children"].to_string();
    assert!(kids.contains("wake each other"), "{kids}");
    assert!(kids.contains("--max-deltas"), "{kids}");
}

#[test]
fn shuffle_seed_is_printed() {
    let o = run("stuck_pingpong", &["--shuffle=7"]);
    let e = clean(&o);
    assert!(
        e.starts_with("note: process order is shuffled with seed 7"),
        "{e}"
    );
    assert!(e.contains("replay it with `--shuffle=7`"), "{e}");
    assert!(e.contains("process order was shuffled with seed 7"), "{e}");
    // Without a value the seed is 1.
    let o = run("stuck_nowait", &["--shuffle", "-Wno-W3102"]);
    assert!(clean(&o).contains("seed 1"), "{}", clean(&o));
}

/// SIGUSR1 prints the report of a running simulation, which then goes on.
#[cfg(unix)]
#[test]
fn sigusr1_prints_a_report_and_the_run_goes_on() {
    use std::io::{BufRead, BufReader, Read};
    let t = tmp("sigusr1");
    let lib = format!("--work=work:{}/lib", t.display());
    assert!(
        yavs(&["analyze", "--std=2008", &lib, "stuck_signal.vhd"])
            .status
            .success()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args([
            "--color",
            "never",
            "run",
            &lib,
            "--stop-time=100sec",
            "stuck_signal",
        ])
        .current_dir(dir())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    // The first report line proves the handler is installed (it is before time 0 ends).
    let mut out = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    out.read_line(&mut line).expect("line");
    assert!(line.contains("started"), "{line}");
    let mut err = child.stderr.take().expect("stderr");
    let pid = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-USR1", &pid])
            .status()
            .unwrap()
            .success()
    );
    let mut text = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !String::from_utf8_lossy(&text).contains("help:") {
        assert!(std::time::Instant::now() < deadline, "no report");
        let n = err.read(&mut buf).expect("read");
        assert!(n > 0, "stderr closed: {}", String::from_utf8_lossy(&text));
        text.extend_from_slice(&buf[..n]);
    }
    // Still running after the report.
    assert!(child.try_wait().expect("wait").is_none());
    child.kill().expect("kill");
    let _ = child.wait();
    let text = String::from_utf8_lossy(&text);
    assert!(
        text.contains("W3101") && text.contains("simulation status at"),
        "{text}"
    );
    assert!(text.contains("stuck_signal.go"), "{text}");
    let _ = std::fs::remove_dir_all(&t);
}

/// With a wave file the first SIGINT only asks the kernel to stop, which it does between
/// cycles; a process that never suspends polls the request at its jump back, so the run ends.
#[cfg(unix)]
#[test]
fn sigint_ends_a_process_that_never_suspends() {
    sigint_ends_spin(None);
}

#[cfg(unix)]
#[test]
fn sigint_ends_a_process_that_never_suspends_interpreted() {
    sigint_ends_spin(Some("--tier=interp"));
}

/// The same under the Cranelift tier (#737, #481).
#[cfg(all(unix, feature = "clift"))]
#[test]
fn sigint_ends_a_process_that_never_suspends_clift() {
    sigint_ends_spin(Some("--tier=clift"));
}

#[cfg(unix)]
fn sigint_ends_spin(tier: Option<&str>) {
    let t = tmp("sigint");
    let lib = format!("--work=work:{}/lib", t.display());
    let wave = format!("--wave={}/o.fst", t.display());
    assert!(
        yavs(&["analyze", "--std=2008", &lib, "stuck_spin.vhd"])
            .status
            .success()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "run"])
        .args(tier)
        .args([&lib, &wave, "stuck_spin"])
        // The bound out of reach, so the run cannot end by itself whatever the speed of the build.
        .env("YAVS_SPIN_LIMIT", "1000000000000000")
        .current_dir(dir())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    // Wait for the run to have started (the wave file exists) before signalling: a fixed
    // delay raced the handler and the process loop on a loaded host (#753).
    let start = std::time::Instant::now();
    while !t.join("o.fst").exists() {
        assert!(child.try_wait().expect("wait").is_none(), "ended by itself");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "the run did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(child.try_wait().expect("wait").is_none(), "ended by itself");
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if std::time::Instant::now() > deadline {
            child.kill().expect("kill");
            let _ = child.wait();
            panic!("the run did not end after SIGINT");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // The same end whatever tier ran the process: reported, exit status 130, file left behind.
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().expect("stdout"), &mut stdout)
        .expect("read");
    assert_eq!(status.code(), Some(130), "{stdout}");
    assert!(stdout.contains("simulation interrupted"), "{stdout}");
    assert!(t.join("o.fst").metadata().expect("wave").len() > 0);
    let _ = std::fs::remove_dir_all(&t);
}

/// The fields of a record pack (#686) are reported one by one: the field waited on has no driver
/// and no event, though another field of the same kernel signal has both. With every history
/// kept; a record that keeps none has the last event of the whole record (#1117).
#[test]
fn a_field_of_a_record_is_reported_alone() {
    let off = [("YAVS_LEAN_EVENTS", "off")];
    let e = snapshot_env("stuck_record", &["--hang-report"], &off, "stuck_record", 0);
    assert!(e.contains("waits for an event on stuck_record.r.b"), "{e}");
    assert!(e.contains("never had an event; it has no driver"), "{e}");
    let e = snapshot("stuck_record", &["--hang-report"], "stuck_record_lean", 0);
    assert!(e.contains("waits for an event on stuck_record.r.b"), "{e}");
    assert!(
        e.contains("last event of the record stuck_record.r at 0 fs; it has no driver"),
        "{e}"
    );
}

/// A delta loop through the fields of one record pack: a record that keeps no history is named
/// once as still changing, one that keeps it by field (#1117).
#[test]
fn a_delta_loop_through_record_fields() {
    let e = snapshot("stuck_record_loop", &[], "stuck_record_loop", 1);
    assert!(e.contains("stuck_record_loop.r still changes"), "{e}");
    assert!(!e.contains("stuck_record_loop.r.a still changes"), "{e}");
    let off = [("YAVS_LEAN_EVENTS", "off")];
    let e = snapshot_env("stuck_record_loop", &[], &off, "stuck_record_loop_full", 1);
    assert!(e.contains("stuck_record_loop.r.a still changes"), "{e}");
}

#[test]
fn hang_report_points_at_the_wait_of_each_process() {
    let e = snapshot(
        "stuck_forever",
        &["--stop-time=1us", "--hang-report"],
        "stuck_forever",
        0,
    );
    // The statement each process stands at, not the first one of its body.
    assert!(e.contains("wait for 100 us"), "{e}");
    assert!(
        e.contains("stuck_forever.clk <= ... (stuck_forever.vhd:6) waits for stuck_forever.clk"),
        "{e}"
    );
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("utf-8")
}

#[test]
fn assert_summary_counts_by_severity_and_place() {
    let plain = run("summary_asserts", &[]);
    let o = run("summary_asserts", &["--assert-summary"]);
    // The reports and the exit status stay; only the table is added.
    assert_eq!(stdout(&plain), stdout(&o));
    assert_eq!(plain.status.code(), o.status.code());
    assert!(plain.stderr.is_empty(), "{}", clean(&plain));
    let e = clean(&o);
    assert!(
        e.starts_with("Assertion summary: 0 failures, 1 error, 2 warnings, 3 notes\n"),
        "{e}"
    );
    for tier in tiers() {
        let e = clean(&run("summary_asserts", &["--assert-summary", tier]));
        for row in [
            "error         1  summary_asserts.vhd:25  first error",
            "warning       2  summary_asserts.vhd:23  too early",
            "note          3  summary_asserts.vhd:22  tick",
        ] {
            assert!(e.contains(row), "{row} in {tier}: {e}");
        }
    }
}

#[test]
fn assert_summary_is_one_json_object_with_message_format_json() {
    let t = tmp("json");
    let lib = format!("--work=work:{}/lib", t.display());
    assert!(
        yavs(&["analyze", "--std=2008", &lib, "summary_asserts.vhd"])
            .status
            .success()
    );
    let o = yavs(&[
        "--message-format=json",
        "run",
        &lib,
        "--assert-summary",
        "summary_asserts",
    ]);
    let _ = std::fs::remove_dir_all(&t);
    let e = clean(&o);
    let j: serde_json::Value = serde_json::from_str(e.trim()).expect(&e);
    let s = &j["assert_summary"];
    assert_eq!(s["totals"]["warning"], 2, "{e}");
    assert_eq!(s["places"][0]["severity"], "error", "{e}");
    assert_eq!(s["places"][0]["line"], 25, "{e}");
}

#[test]
fn activity_lists_the_busiest_processes_and_signals() {
    let t = tmp("activity");
    let file = t.join("a.json");
    let flag = format!("--activity={}", file.display());
    for tier in tiers() {
        let o = run("summary_asserts", &[&flag, tier]);
        assert_eq!(o.status.code(), Some(1), "{}", clean(&o));
        let e = clean(&o);
        // A clock of three periods: six events, and the process of the clock runs for each and at the start.
        assert!(e.contains("Process activations: "), "{e}");
        assert!(e.contains("summary_asserts.clk"), "{e}");
        let row = e
            .lines()
            .find(|l| l.contains("summary_asserts.clk  "))
            .expect(&e);
        assert!(row.trim_start().starts_with("6 "), "{e}");
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("json")).expect("json");
        assert_eq!(j["signals"][0]["name"], "summary_asserts.clk", "{j}");
        assert_eq!(j["signals"][0]["events"], 6, "{j}");
        assert!(j["processes"][0]["runs"].as_u64() >= Some(4), "{j}");
    }
    let bad = run("summary_asserts", &["--activity=/nonexistent/dir/a.json"]);
    assert!(clean(&bad).contains("E0124"), "{}", clean(&bad));
    let _ = std::fs::remove_dir_all(&t);
}
