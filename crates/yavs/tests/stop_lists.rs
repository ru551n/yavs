//! The compilation cache's list of compiled functions is on disk when a run ends by any route
//! (normal end, `stop`, `finish`, a failing assertion, `--stop-time`, Ctrl-C, SIGTERM, standard input
//! closing), with the compile thread waiting for a compile (`--tier-compile=wait`) and alongside the
//! run (`background`); the next run links what was compiled instead of compiling it again. A first
//! version stored the list only when the compile thread was idle, and a run that ended by a signal
//! (the signal's thread exits the process) or under a compile in flight left none.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One call of `mix` is compiled before it runs (`--tier-heat=1,0,0`); `ROUTE` ends the run.
const SRC: &str = "use std.env.all;
entity sl is end;
architecture a of sl is
  function mix(x : integer; y : integer) return integer is
    variable v : integer := x;
  begin
    for i in 0 to 15 loop
      v := (v * 31 + y + i) mod 1000003;
    end loop;
    return v;
  end;
begin
  process
    variable acc : integer := 0;
  begin
    acc := mix(acc, 7);
    report \"acc \" & integer'image(acc);
    ROUTE
  end process;
end;
";

const FOREVER: &str = "loop wait for 1 ns; end loop;";

fn yavs(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(["--color", "never"])
        .args(args)
        .env("YAVS_AUTO_TRACE", "1")
        .env_remove("YAVS_AUTO_SYNC")
        .env_remove("YAVS_AUTO_POLICY");
    c
}

/// An analysed design whose process ends with `route`; the work library's directory.
fn design(name: &str, route: &str) -> (common::Tmp, String) {
    let dir = tempdir(name);
    std::fs::write(dir.join("sl.vhd"), SRC.replace("ROUTE", route)).expect("write");
    let lib = format!("--work=work:{}", dir.join("lib").display());
    let a = yavs(&[
        "analyze",
        "--std=2008",
        &lib,
        &dir.join("sl.vhd").display().to_string(),
    ])
    .output()
    .expect("analyze");
    assert!(a.status.success(), "{a:?}");
    (dir, lib)
}

fn run(lib: &str, mode: &str, extra: &[&str]) -> Command {
    let mut c = yavs(
        &[
            &[
                "run",
                "--std=2008",
                lib,
                "--tier=auto",
                "--tier-heat=1,0,0",
                mode,
            ][..],
            extra,
            &["sl"],
        ]
        .concat(),
    );
    c.stdin(Stdio::null());
    c
}

/// The files of the objects store under `dir` named `kind-...`.
fn entries(dir: &Path, kind: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{kind}-")))
            {
                out.push(p);
            }
        }
    }
    out
}

/// What the tier made of each function of a run (`YAVS_AUTO_TRACE`): `true` for a link from the
/// cache.
fn compiled(stderr: &[u8]) -> Vec<bool> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|l| l.contains("compiled `"))
        .map(|l| l.contains(" hit "))
        .collect()
}

/// The next run of `lib` (ended by `extra`, if the design does not end): it links every function
/// the first run compiled.
fn links_all(lib: &str, mode: &str, extra: &[&str], what: &str) {
    let o = run(lib, mode, extra).output().expect("run");
    let c = compiled(&o.stderr);
    assert!(
        !c.is_empty() && c.iter().all(|&hit| hit),
        "{what}: the next run did not link what the first compiled: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn wait_for_end(mut child: Child, what: &str) {
    let t = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if t.elapsed() > Duration::from_secs(60) {
            child.kill().expect("kill");
            panic!("{what}: the run did not end");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_run_that_ends_by_any_route_leaves_the_list() {
    let routes: [(&str, &str, &[&str]); 5] = [
        ("the end of the process", "wait;", &[]),
        ("std.env.stop", "stop;", &[]),
        ("std.env.finish", "finish;", &[]),
        (
            "a failing assertion",
            "assert false report \"x\" severity failure; wait;",
            &[],
        ),
        ("--stop-time", FOREVER, &["--stop-time=5ns"]),
    ];
    for mode in ["--tier-compile=wait", "--tier-compile=background"] {
        for (what, route, extra) in routes {
            let what = format!("{what}, {mode}");
            let (dir, lib) = design("stop_lists", route);
            let first = run(&lib, mode, extra).output().expect("run");
            let c = compiled(&first.stderr);
            assert!(
                !c.is_empty() && !c.contains(&true),
                "{what}: nothing compiled in the first run: {}",
                String::from_utf8_lossy(&first.stderr)
            );
            assert!(!entries(&dir, "list").is_empty(), "{what}: no list stored");
            links_all(&lib, mode, extra, &what);
        }
    }
}

/// Starts a run that never ends and returns it once it has compiled `mix` and run past it.
fn running(lib: &str, mode: &str, extra: &[&str]) -> Child {
    let mut child = run(lib, mode, extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start yavs");
    // The report after the call: the function is compiled and noted by then (eager).
    let mut out = BufReader::new(child.stdout.as_mut().expect("stdout"));
    let mut line = String::new();
    while !line.contains("acc ") {
        line.clear();
        assert!(
            out.read_line(&mut line).expect("read") > 0,
            "the run ended early"
        );
    }
    child
}

#[cfg(unix)]
#[test]
fn a_run_ended_by_a_signal_leaves_the_list() {
    for mode in ["--tier-compile=wait", "--tier-compile=background"] {
        for sig in ["-INT", "-TERM"] {
            let what = format!("{sig}, {mode}");
            let (dir, lib) = design("stop_lists_sig", FOREVER);
            let child = running(&lib, mode, &[]);
            let st = Command::new("kill")
                .args([sig, &child.id().to_string()])
                .status()
                .expect("kill");
            assert!(st.success());
            wait_for_end(child, &what);
            assert!(!entries(&dir, "list").is_empty(), "{what}: no list stored");
            links_all(&lib, mode, &["--stop-time=5ns"], &what);
        }
    }
}

#[test]
fn a_run_ended_by_its_closing_stdin_leaves_the_list() {
    for mode in ["--tier-compile=wait", "--tier-compile=background"] {
        let (dir, lib) = design("stop_lists_stdin", FOREVER);
        let mut child = running(&lib, mode, &["--exit-on-stdin-close"]);
        drop(child.stdin.take());
        wait_for_end(child, mode);
        assert!(!entries(&dir, "list").is_empty(), "{mode}: no list stored");
        links_all(&lib, mode, &["--stop-time=5ns"], mode);
    }
}
