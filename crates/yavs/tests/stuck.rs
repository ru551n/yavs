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
    let t = tmp(name);
    let lib = format!("--work=work:{}/lib", t.display());
    let file = format!("{name}.vhd");
    assert!(
        yavs(&["analyze", "--std=2008", &lib, &file])
            .status
            .success()
    );
    let o = yavs(&[&["run", &lib][..], extra, &[name]].concat());
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
    let o = run(name, extra);
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
    assert!(e.starts_with("note: process order is shuffled with seed 7"), "{e}");
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
        .args(["--color", "never", "run", &lib, "--stop-time=100sec", "stuck_signal"])
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
    assert!(Command::new("kill").args(["-USR1", &pid]).status().unwrap().success());
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
    assert!(text.contains("W3101") && text.contains("simulation status at"), "{text}");
    assert!(text.contains(":stuck_signal:go"), "{text}");
    let _ = std::fs::remove_dir_all(&t);
}
