//! Snapshot tests for `yavs analyze` (exit code and stderr), in the style of diag.rs.
//!
//! Cases are listed below; sources are in `tests/cli`, expected output in
//! `tests/cli/NAME.stderr` with the scratch directory shown as `$TMP`.
//! `YAVS_BLESS=1` rewrites the expected files.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

struct Case {
    name: &'static str,
    /// Arguments; `$TMP` is the scratch directory, which also holds `lib/`.
    args: &'static [&'static str],
    code: i32,
}

const CASES: &[Case] = &[
    Case {
        name: "analyze_ok",
        args: &["analyze", "--work=work:$TMP/lib", "ok.vhd"],
        code: 0,
    },
    Case {
        name: "analyze_error",
        args: &["analyze", "--work=work:$TMP/lib", "bad.vhd"],
        code: 1,
    },
    Case {
        name: "encrypted",
        args: &[
            "analyze",
            "--std=2008",
            "--work=work:$TMP/lib",
            "encrypted.vhd",
        ],
        code: 1,
    },
    Case {
        name: "missing_library",
        args: &["analyze", "--work=work:$TMP/lib", "nolib.vhd"],
        code: 1,
    },
    Case {
        name: "map_ieee",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "--map=ieee:$TMP/x",
            "ok.vhd",
        ],
        code: 2,
    },
    Case {
        name: "bad_mapping",
        args: &["analyze", "--work=work", "ok.vhd"],
        code: 2,
    },
    Case {
        name: "bad_std",
        args: &["analyze", "--std=87", "--work=work:$TMP/lib", "ok.vhd"],
        code: 2,
    },
    Case {
        name: "response_file",
        args: &["analyze", "@$TMP/args.rsp"],
        code: 0,
    },
    Case {
        name: "response_file_missing",
        args: &["analyze", "@$TMP/nope.rsp"],
        code: 2,
    },
    Case {
        name: "batch_ok",
        args: &["analyze", "--work=work:$TMP/lib", "ok.vhd", "ok2.vhd"],
        code: 0,
    },
    Case {
        name: "batch_fail",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "ok.vhd",
            "bad.vhd",
            "ok2.vhd",
        ],
        code: 1,
    },
    Case {
        name: "batch_fail_json",
        args: &[
            "--message-format=json",
            "analyze",
            "--work=work:$TMP/lib",
            "ok.vhd",
            "bad.vhd",
            "ok2.vhd",
        ],
        code: 1,
    },
    Case {
        name: "batch_fail_short",
        args: &[
            "--message-format=short",
            "analyze",
            "--work=work:$TMP/lib",
            "ok.vhd",
            "bad.vhd",
            "ok2.vhd",
        ],
        code: 1,
    },
    Case {
        name: "batch_first_fails",
        args: &["analyze", "--work=work:$TMP/lib", "bad.vhd", "ok.vhd"],
        code: 1,
    },
    Case {
        name: "batch_filelist",
        args: &["analyze", "--work=work:$TMP/lib", "@$TMP/files.lst"],
        code: 0,
    },
    Case {
        name: "empty_mapped_library",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "--map=other:$TMP/none",
            "emptylib.vhd",
        ],
        code: 1,
    },
    Case {
        name: "work_std",
        args: &["analyze", "--work=std:$TMP/lib", "ok.vhd"],
        code: 2,
    },
    Case {
        name: "warn_plain",
        args: &["analyze", "--work=work:$TMP/lib", "warn.vhd"],
        code: 0,
    },
    Case {
        name: "warn_werror",
        args: &["analyze", "--work=work:$TMP/lib", "-Werror", "warn.vhd"],
        code: 1,
    },
    Case {
        name: "psl_skipped",
        args: &["analyze", "--work=work:$TMP/lib", "psl_run.vhd"],
        code: 0,
    },
    Case {
        name: "psl_formal_only",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "-Werror=W0004",
            "psl_formal_only.vhd",
        ],
        code: 0,
    },
    Case {
        name: "psl_werror",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "-Werror=W0004",
            "psl_run.vhd",
        ],
        code: 1,
    },
    Case {
        name: "warn_deny_code",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "-Werror=W0001",
            "warn.vhd",
        ],
        code: 1,
    },
    Case {
        name: "warn_allow_code",
        args: &["analyze", "--work=work:$TMP/lib", "-Wno-W0001", "warn.vhd"],
        code: 0,
    },
    Case {
        name: "warn_code_after_werror",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "-Werror",
            "-WW0001",
            "warn.vhd",
        ],
        code: 0,
    },
    Case {
        name: "warn_allow_after_deny",
        args: &[
            "analyze",
            "--work=work:$TMP/lib",
            "-Werror=W0001",
            "-Wno-W0001",
            "warn.vhd",
        ],
        code: 0,
    },
    Case {
        name: "warn_bad_code",
        args: &["analyze", "--work=work:$TMP/lib", "-Wno-W9998", "warn.vhd"],
        code: 2,
    },
    Case {
        name: "warn_bad_form",
        args: &["analyze", "--work=work:$TMP/lib", "-Wfoo", "warn.vhd"],
        code: 2,
    },
    Case {
        name: "warn_elab_bad_form",
        args: &["elab", "--work=work:$TMP/lib", "-Wno-nothing", "top"],
        code: 2,
    },
    Case {
        name: "encoding_auto",
        args: &["analyze", "--work=work:$TMP/lib", "latin1.vhd"],
        code: 0,
    },
    Case {
        name: "encoding_latin1",
        args: &[
            "--encoding=latin1",
            "analyze",
            "--work=work:$TMP/lib",
            "latin1.vhd",
        ],
        code: 0,
    },
    Case {
        name: "encoding_utf8",
        args: &[
            "--encoding=utf8",
            "analyze",
            "--work=work:$TMP/lib",
            "latin1.vhd",
        ],
        code: 1,
    },
    Case {
        name: "json",
        args: &[
            "--message-format=json",
            "analyze",
            "--work=work:$TMP/lib",
            "bad.vhd",
        ],
        code: 1,
    },
];

fn run(case: &Case, dir: &Path, tmp: &Path) -> (i32, String) {
    let t = tmp.to_str().expect("utf-8 tmp");
    std::fs::write(
        tmp.join("args.rsp"),
        format!("--std=2008\r\n--work=work:{t}/lib\r\nok.vhd"),
    )
    .expect("rsp");
    std::fs::write(tmp.join("files.lst"), "ok.vhd\nok2.vhd\n").expect("lst");
    let args: Vec<String> = case.args.iter().map(|a| a.replace("$TMP", t)).collect();
    let out = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(&args)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .output()
        .expect("run yavs");
    let err = String::from_utf8(out.stderr)
        .expect("utf-8")
        .replace(t, "$TMP")
        .replace(
            "The system cannot find the file specified.",
            "No such file or directory",
        );
    (out.status.code().expect("exit code"), err)
}

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let bless = std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1");
    let mut failed = Vec::new();
    for case in CASES {
        let tmp = tempdir(case.name);
        let (code, err) = run(case, &dir, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        let path = dir.join(format!("{}.stderr", case.name));
        if bless {
            std::fs::write(&path, &err).expect("write");
        } else if code != case.code || err != std::fs::read_to_string(&path).unwrap_or_default() {
            failed.push(format!(
                "--- {} (exit {code}, want {}):\n{err}",
                case.name, case.code
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A design with PSL elaborates and runs: PSL outside the subset is skipped (#138), the
/// subset runs as checker processes (#226), a `-- psl` pragma included.
#[test]
fn psl_run() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("psl_run");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run")
    };
    let analyze =
        |extra: &[&str]| yavs(&[&["analyze", &lib][..], extra, &["psl_run.vhd"]].concat());
    // Without --psl the pragma lines are comments: one cover hit fewer.
    assert!(analyze(&[]).status.success());
    let o = yavs(&["run", &lib, "psl_run"]);
    let plain = String::from_utf8(o.stdout).expect("utf-8");
    assert_eq!(plain.matches("cover hit").count(), 0, "{plain}");
    assert!(analyze(&["--psl"]).status.success());
    let o = yavs(&["run", "--psl", &lib, "psl_run"]);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8(o.stdout).expect("utf-8");
    assert!(out.contains("design with PSL ran"), "{out}");
    assert_eq!(out.matches("cover hit").count(), 5, "{out}");
}

/// A runtime error is a rustc-style diagnostic on stderr (#84): the kernel's code, the failing
/// statement, one label per calling frame, and the time, delta and process.
#[test]
fn runtime_error() {
    run_snapshot("runtime_error", 1);
}

/// The same labels when thin subprograms are inlined into their callers (`opt/inline.rs`): the
/// frames of the inlined functions are listed as before.
#[test]
fn runtime_error_in_thin_subprograms() {
    run_snapshot("runtime_error_thin", 1);
}

/// Reports are written in blocks on a pipe, but a diagnostic on the same pipe still comes after
/// every report that preceded it (#428).
#[test]
fn output_order_on_one_pipe() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("output_order");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.args(["--color", "never"]).current_dir(&dir);
        c
    };
    let a = yavs()
        .args(["analyze", &lib, "output_order.vhd"])
        .status()
        .expect("run");
    assert!(a.success());
    let (mut r, w) = std::io::pipe().expect("pipe");
    let mut c = yavs();
    c.args(["run", &lib, "output_order"])
        .stdout(w.try_clone().expect("dup"))
        .stderr(w);
    let mut child = c.spawn().expect("spawn");
    // The command holds the write ends: drop it so that the read ends at the child's exit.
    drop(c);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut r, &mut text).expect("read");
    assert_eq!(child.wait().expect("wait").code(), Some(1));
    let _ = std::fs::remove_dir_all(&tmp);
    let reports: Vec<_> = text.match_indices("** Note: ").collect();
    assert_eq!(reports.len(), 2000);
    let err = text.find("error[E3009]").expect("diagnostic");
    assert!(reports.iter().all(|(at, _)| *at < err), "{text}");
    assert!(text.contains("line 2000\n"));
}

/// SIGINT and SIGTERM write out the reports that are still waiting in the output buffer before
/// the run ends (#428).
#[cfg(unix)]
#[test]
fn signal_keeps_reports() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("signal_flush");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", &lib, "signal_flush.vhd"])
        .current_dir(&dir)
        .status()
        .expect("run");
    assert!(a.success());
    for sig in ["INT", "TERM"] {
        let ready = tmp.join(format!("ready-{sig}"));
        let out = tmp.join(format!("out-{sig}"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["run", &lib, &format!("-gready={}", ready.display())])
            .arg("signal_flush")
            .current_dir(&dir)
            .stdout(std::fs::File::create(&out).expect("out"))
            .spawn()
            .expect("spawn");
        let t0 = std::time::Instant::now();
        while !ready.exists() {
            assert!(t0.elapsed().as_secs() < 60, "the run never got going");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pid = child.id().to_string();
        let k = Command::new("kill")
            .args([&format!("-{sig}"), &pid])
            .status();
        assert!(k.expect("kill").success());
        assert!(!child.wait().expect("wait").success());
        let text = std::fs::read_to_string(&out).expect("read");
        assert!(text.contains("before signal"), "{sig}: {text:?}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The output of a run that goes quiet is written out within a fraction of a second, with no
/// signal: the tick of the signal thread (#562).
#[cfg(unix)]
#[test]
fn quiet_run_is_flushed() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("quiet_flush");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", &lib, "signal_flush.vhd"])
        .current_dir(&dir)
        .status()
        .expect("run");
    assert!(a.success());
    let (ready, out) = (tmp.join("ready"), tmp.join("out"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["run", &lib, &format!("-gready={}", ready.display())])
        .arg("signal_flush")
        .current_dir(&dir)
        .stdout(std::fs::File::create(&out).expect("out"))
        .spawn()
        .expect("spawn");
    let t0 = std::time::Instant::now();
    let mut seen = false;
    while !seen && t0.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        // The report is made before the marker file exists; the process then computes for ever.
        seen = ready.exists()
            && std::fs::read_to_string(&out).is_ok_and(|t| t.contains("before signal"));
    }
    child.kill().expect("kill");
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(seen, "the report was not written out while the run went on");
}

/// Two signals end a run that is stuck inside one cycle: the second exits with status 1
/// after writing out the reports, also when they come in one burst (#562: one thread counts
/// them), with a waveform file or `--profile` (whose first signal only asks the run to stop
/// between cycles; in a build with the compiled tier the exit used to crash, #625).
#[cfg(unix)]
#[test]
fn second_signal_exits_at_once() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("second_signal");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", &lib, "signal_flush.vhd"])
        .current_dir(&dir)
        .status()
        .expect("run");
    assert!(a.success());
    for (round, extra) in [
        format!("--wave={}", tmp.join("w.fst").display()),
        "--profile".to_string(),
    ]
    .into_iter()
    .cycle()
    .take(6)
    .enumerate()
    {
        let (ready, out) = (tmp.join(format!("ready{round}")), tmp.join("out"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["run", &extra, &lib, &format!("-gready={}", ready.display())])
            .arg("signal_flush")
            .current_dir(&dir)
            .stdout(std::fs::File::create(&out).expect("out"))
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let t0 = std::time::Instant::now();
        while !ready.exists() {
            assert!(t0.elapsed().as_secs() < 60, "the run never got going");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let pid = child.id().to_string();
        // Two signals at once; if the kernel merged them, one more every 150 ms until it ends.
        let kill = |pids: &[&str]| {
            let k = Command::new("kill").arg("-INT").args(pids).status();
            assert!(k.expect("kill").success());
        };
        kill(&[&pid, &pid]);
        let t0 = std::time::Instant::now();
        let status = loop {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if let Some(s) = child.try_wait().expect("wait") {
                break s;
            }
            if t0.elapsed().as_secs() > 20 {
                child.kill().expect("kill");
                panic!("two signals did not end the run");
            }
            kill(&[&pid]);
        };
        assert_eq!(status.code(), Some(1), "{extra}, round {round}: {status:?}");
        let text = std::fs::read_to_string(&out).expect("read");
        assert!(text.contains("before signal"), "{text:?}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Runs `yavs run` on `long_report` with a piped stdout; `extra` are more arguments.
#[cfg(unix)]
fn spawn_long_report(tmp: &Path, extra: &[&str]) -> std::process::Child {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", &lib, "long_report.vhd"])
        .current_dir(&dir)
        .status()
        .expect("run");
    assert!(a.success());
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["run", &lib])
        .args(extra)
        .arg("long_report")
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn")
}

/// A process that reports without ever waiting shows its reports as it makes them, also when
/// it is killed (#483): the first lines reach a pipe within about a second, and SIGTERM keeps
/// what was written.
#[cfg(unix)]
#[test]
fn long_process_reports_live() {
    use std::io::{BufRead, BufReader};
    for threads in ["--threads=1", "--threads=4"] {
        let tmp = tempdir("long_report_live");
        let mut child = spawn_long_report(&tmp, &[threads]);
        let (tx, rx) = std::sync::mpsc::channel();
        let out = child.stdout.take().expect("stdout");
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines() {
                if tx.send(l.expect("line")).is_err() {
                    break;
                }
            }
        });
        let t = std::time::Duration::from_secs(30);
        let Ok(first) = rx.recv_timeout(t) else {
            child.kill().expect("kill");
            panic!("{threads}: no report in 30 s");
        };
        assert!(first.contains("line 1"), "{threads}: {first:?}");
        let pid = child.id().to_string();
        let k = Command::new("kill").args(["-TERM", &pid]).status();
        assert!(k.expect("kill").success());
        assert!(!child.wait().expect("wait").success());
        let mut n = 1;
        while rx.recv_timeout(t).is_ok() {
            n += 1;
        }
        assert!(n >= 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// A closed stdout pipe ends the run with status 141 and a message on stderr, and the
/// waveform file is still finished (#484).
#[cfg(unix)]
#[test]
fn closed_stdout_stops_run() {
    use std::io::{BufRead, BufReader, Read};
    let tmp = tempdir("long_report_pipe");
    let wave = tmp.join("w.fst");
    let mut child = spawn_long_report(
        &tmp,
        &["-gwaiting=true", &format!("--wave={}", wave.display())],
    );
    let mut out = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    out.read_line(&mut line).expect("line");
    assert!(line.contains("line"), "{line:?}");
    drop(out);
    let t0 = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if t0.elapsed().as_secs() > 20 {
            child.kill().expect("kill");
            panic!("the run did not stop after its stdout was closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(141));
    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("stderr");
    assert!(err.contains("standard output"), "{err:?}");
    assert!(std::fs::metadata(&wave).expect("wave").len() > 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `to_string`, `'image` and `to_hstring` of vectors, strings, records and arrays of enumerations
/// give the text the LRM says (#428); the expected lines are `tests/cli/image_text.stdout`.
#[test]
fn image_text() {
    run_stdout("image_text", 0);
}

/// READ and WRITE edit a line in place, but an access value copied before the edit is stale
/// afterwards: using it is a runtime error (#428).
#[test]
fn line_edit_in_place() {
    run_stdout("line_edit", 1);
}

/// Analyze `tests/cli/NAME.vhd` (2019), run its entity `NAME`, check the exit code and compare
/// the notes it reports at time zero with the lines of `NAME.stdout`.
fn run_stdout(name: &str, code: i32) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir(name);
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run")
    };
    let file = format!("{name}.vhd");
    assert!(
        yavs(&["analyze", "--std=2019", &lib, &file])
            .status
            .success()
    );
    let o = yavs(&["run", &lib, name]);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(o.status.code(), Some(code));
    let out = String::from_utf8(o.stdout).expect("utf-8");
    let lines: String = out
        .lines()
        .filter_map(|l| l.strip_prefix("** Note: 0fs+0: "))
        .map(|l| format!("{l}\n"))
        .collect();
    let want = std::fs::read_to_string(dir.join(format!("{name}.stdout"))).expect("expected file");
    assert_eq!(lines, want);
}

/// A runtime error while packages are initialised is an elaboration error (exit 2) with the
/// same diagnostic, not an internal error (#110).
#[test]
fn init_error() {
    run_snapshot("init_error", 2);
}

/// A package declaration's constant calling a function of its own package is E2114 even when
/// only a process reads it (#197, LRM 14.4.2).
#[test]
fn body_not_elaborated() {
    run_snapshot("body_not_elaborated", 2);
}

/// An external name whose subtype indication has another length than its object is E2121
/// (LRM 8.7, #463).
#[test]
fn external_mismatch() {
    run_snapshot("external_mismatch", 2);
}

/// A function calling a procedure of another unit that waits is E1801 at elaboration, not
/// an internal error (LRM 10.2, #823).
#[test]
fn function_calls_waiting_procedure() {
    run_snapshot("function_calls_waiting_procedure", 2);
}

/// Analyze `tests/cli/NAME.vhd`, run its entity `NAME`, check the exit code and compare stderr
/// with `NAME.stderr`.
fn run_snapshot(name: &str, code: i32) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir(name);
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run")
    };
    let file = format!("{name}.vhd");
    assert!(yavs(&["analyze", &lib, &file]).status.success());
    let o = yavs(&["run", &lib, name]);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(o.status.code(), Some(code));
    let canon = dir.canonicalize().expect("dir");
    // Windows: no `\\?\` prefix in the diagnostic, and `\` after the directory.
    let canon = canon.to_str().expect("utf-8 dir");
    let canon = canon.strip_prefix(r"\\?\").unwrap_or(canon);
    let err = String::from_utf8(o.stderr)
        .expect("utf-8")
        .replace(canon, "$DIR")
        .replace(r"$DIR\", "$DIR/");
    let path = dir.join(format!("{name}.stderr"));
    if std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1") {
        std::fs::write(&path, &err).expect("write");
    } else {
        assert_eq!(err, std::fs::read_to_string(&path).unwrap_or_default());
    }
}

/// A second analyze into the same library keeps both files; a read-only library fails cleanly.
#[test]
fn library_recorded_and_read_only() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let tmp = tempdir("lib");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |f: &str| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never", "analyze", &lib, f])
            .current_dir(&dir)
            .output()
            .expect("run")
    };
    assert_eq!(yavs("ok.vhd").status.code(), Some(0));
    let m = std::fs::read_to_string(tmp.join("lib/yavs-lib.toml")).expect("manifest");
    assert!(m.contains("name = \"work\""), "{m}");
    assert!(
        m.contains("std = \"2008\"") && m.contains("[\"package\", \"ok\"]"),
        "{m}"
    );

    // A batch into an existing library reads its manifest while holding its lock.
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "analyze", &lib, "ok.vhd", "ok2.vhd"])
        .current_dir(&dir)
        .output()
        .expect("run");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let mut perm = std::fs::metadata(tmp.join("lib"))
        .expect("meta")
        .permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(tmp.join("lib"), perm.clone()).expect("chmod");
    let writable = std::fs::File::create(tmp.join("lib/probe")).is_ok(); // root ignores modes
    if !writable {
        let o = yavs("ok.vhd");
        assert_eq!(o.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&o.stderr).contains("error[L00"));
    }
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    let _ = std::fs::set_permissions(tmp.join("lib"), perm);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Two processes analyzing batches into one library at the same time lose nothing
/// (plan rule 14), and a batch records its files in order.
#[test]
fn concurrent_batches() {
    let tmp = tempdir("conc");
    let src = tmp.join("src");
    std::fs::create_dir_all(&src).expect("src");
    let n = 25;
    let files = |p: &str| -> Vec<String> {
        (0..n)
            .map(|i| {
                let f = src.join(format!("{p}{i}.vhd"));
                std::fs::write(
                    &f,
                    format!("package {p}{i} is\n  constant c : integer := {i};\nend package;\n"),
                )
                .expect("write");
                f.display().to_string()
            })
            .collect()
    };
    let lib = format!("--work=work:{}/lib", tmp.display());
    let spawn = |fs: Vec<String>| {
        let lib = lib.clone();
        std::thread::spawn(move || {
            Command::new(env!("CARGO_BIN_EXE_yavs"))
                .args(["--color", "never", "analyze", &lib])
                .args(fs)
                .output()
                .expect("run")
        })
    };
    let (ha, hb) = (spawn(files("pa")), spawn(files("pb")));
    for h in [ha, hb] {
        let o = h.join().expect("join");
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    let m = std::fs::read_to_string(tmp.join("lib/yavs-lib.toml")).expect("manifest");
    for p in ["pa", "pb"] {
        for i in 0..n {
            assert!(
                m.contains(&format!("[\"package\", \"{p}{i}\"]")),
                "{p}{i} missing\n{m}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A file the design never closes is written out when the run ends, also when `yavs run` ends
/// without freeing the design.
#[test]
fn files_left_open_are_written() {
    let tmp = tempdir("openfile");
    std::fs::write(
        tmp.join("f.vhd"),
        "use std.textio.all;\n\
         entity f is end;\n\
         architecture t of f is begin\n\
           process\n\
             file o : text open write_mode is \"out.txt\";\n\
             variable l : line;\n\
           begin\n\
             write(l, string'(\"hello\")); writeline(o, l);\n\
             wait;\n\
           end process;\n\
         end;\n",
    )
    .expect("write");
    for args in [
        &["analyze", "--work=work:lib", "f.vhd"][..],
        &["run", "--work=work:lib", "f"],
    ] {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&tmp)
            .args(args)
            .output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    assert_eq!(
        std::fs::read_to_string(tmp.join("out.txt")).expect("out.txt"),
        "hello\n"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `--tier=auto` (#125): whenever the hot functions are compiled and swapped in, the output,
/// a file the design leaves open, and the text of a runtime error in compiled code are the
/// interpreter's (rule 5), at 1 and 4 threads.
#[cfg(feature = "llvm")]
#[test]
fn tier_auto_matches_interp() {
    let tmp = tempdir("tierauto");
    std::fs::write(
        tmp.join("hot.vhd"),
        "use std.textio.all;\n\
         entity hot is end;\n\
         architecture t of hot is\n\
           function mix(a, b : integer) return integer is\n\
           begin assert a >= 0 report \"mix\"; return (a * 31 + b) mod 1000003; end function;\n\
           signal s : integer := 0;\n\
         begin\n\
           p0 : process\n\
             file f : text open write_mode is \"out.txt\";\n\
             variable l : line;\n\
             variable acc : integer := 0;\n\
           begin\n\
             for i in 1 to 20000 loop\n\
               acc := mix(acc, i);\n\
               if i mod 5000 = 0 then\n\
                 write(l, acc); writeline(f, l);\n\
                 report \"acc=\" & integer'image(acc);\n\
               end if;\n\
             end loop;\n\
             s <= acc;\n\
             wait for 1 ns;\n\
             acc := 2;\n\
             for i in 1 to 100 loop acc := acc * 3; end loop;\n\
             wait;\n\
           end process;\n\
           p1 : process (s) begin report \"s=\" & integer'image(s); end process;\n\
         end;\n",
    )
    .expect("write");
    let yavs = |args: &[&str]| {
        let _ = std::fs::remove_file(tmp.join("out.txt"));
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&tmp)
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        let file = std::fs::read_to_string(tmp.join("out.txt")).unwrap_or_default();
        (
            o.status.code().expect("exit code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            file,
        )
    };
    assert_eq!(yavs(&["analyze", "--work=work:lib", "hot.vhd"]).0, 0);
    let interp = yavs(&["run", "--work=work:lib", "hot", "--tier=interp"]);
    assert_eq!(interp.0, 1, "{}", interp.1);
    assert!(interp.1.contains("cannot be represented"), "{}", interp.1);
    assert_eq!(interp.3.lines().count(), 4, "{}", interp.3);
    for heat in [
        "--tier-heat=1,0,0",
        "--tier-heat=2000,10,12",
        "--tier-heat=1000000,20000,12",
    ] {
        for threads in ["--threads=1", "--threads=4"] {
            let args = [
                "run",
                "--work=work:lib",
                "hot",
                "--tier=auto",
                heat,
                threads,
            ];
            assert_eq!(yavs(&args), interp, "{heat} {threads}");
        }
    }
    // Compiled from the first call or jump: both functions run compiled.
    let (_, err, _, _) = yavs(&[
        "run",
        "--work=work:lib",
        "hot",
        "--tier=auto",
        "--tier-heat=1,0,0",
        "--verbose",
    ]);
    for f in ["`mix`", "`p0`"] {
        assert!(
            err.contains(&format!("note: {f}")) && err.contains("is hot: compiled"),
            "{err}"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A waveform whose value comes from a call (#542): the interpreter's fast loop drops the
/// waveform's setup and fuses the `Copy` and `Drive` after the call, so compiled code taking
/// over at the call's return (every function compiled from its first call) must not need it.
#[cfg(feature = "llvm")]
#[test]
fn tier_auto_call_between_waveform_and_drive() {
    let tmp = tempdir("tierwave");
    std::fs::write(
        tmp.join("w.vhd"),
        "entity w is end;\n\
         architecture t of w is\n\
           function shl(x : bit_vector) return bit_vector is\n\
           begin return x(x'left + 1 to x'right) & '1'; end function;\n\
           signal s : bit_vector(0 to 3) := \"0000\";\n\
         begin\n\
           process begin\n\
             s <= shl(s);\n\
             wait for 1 ns;\n\
             report to_string(s);\n\
             wait;\n\
           end process;\n\
         end;\n",
    )
    .expect("write");
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&tmp)
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        (
            o.status.code().expect("exit code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    assert_eq!(yavs(&["analyze", "--work=work:lib", "w.vhd"]).0, 0);
    let interp = yavs(&["run", "--work=work:lib", "w", "--tier=interp"]);
    assert_eq!(interp.0, 0, "{}", interp.1);
    assert!(interp.2.contains("0001"), "{}", interp.2);
    let auto = yavs(&[
        "run",
        "--work=work:lib",
        "w",
        "--tier=auto",
        "--tier-heat=1,0,0",
    ]);
    assert_eq!(auto, interp);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `yavs elab` and `yavs run` (cli-contract section 5): exit codes and diagnostic codes.
#[test]
fn elab_and_run() {
    let tmp = tempdir("elab");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "entity leaf is generic (W : natural := 1); end;\n\
         architecture a of leaf is signal s : bit_vector(W - 1 downto 0); begin end;\n\
         entity top is generic (N : positive := 2; MSG : string := \"x\"); end;\n\
         architecture t of top is begin u : entity work.leaf generic map (W => N); end;\n\
         entity busy is end;\n\
         architecture t of busy is begin process begin report \"hi\"; wait for 1 ns;\n\
           report \"bad\" severity error; wait for 1 ns; report \"after\"; wait; end process; end;\n\
         use std.env.all; entity fin is end;\n\
         architecture t of fin is begin process begin wait for 3 ns; finish(3);\n           report \"unreachable\"; wait; end process; end;\n",
    )
    .expect("write");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        (
            o.status.code().expect("exit code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let s = src.display().to_string();
    assert_eq!(yavs(&["analyze", &lib, &s]).0, 0);
    // Generics after TOP, the long verbatim form, repeated options (last wins).
    let (code, err, out) = yavs(&[
        "elab",
        &lib,
        "--print-hierarchy",
        "top.t",
        "-gN=3",
        "-gMSG=enabled_test_cases : a,b/c\\d.e",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains(":top:u:s"), "{out}");
    let rsp = tmp.join("args.rsp");
    std::fs::write(&rsp, format!("{lib}\ntop\n-gN=4\n-gnope=1\n")).expect("rsp");
    let (code, err, _) = yavs(&["elab", &format!("@{}", rsp.display())]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("warning[W2105]"), "{err}");
    let (code, err, _) = yavs(&["elab", &lib, "top", "-gN=0"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E2104]"), "{err}");
    let (code, err, _) = yavs(&["elab", &lib, "nothing"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E0104]"), "{err}");
    // A design without processes runs to event starvation.
    let (code, err, _) = yavs(&["run", &lib, "--exit-severity=error", "top"]);
    assert_eq!(code, 0, "{err}");
    // Reports in sim1's format on stdout. By default an error report does not stop the run,
    // which still exits 1 (user decision, cli-contract section 5).
    let (code, err, out) = yavs(&["run", &lib, "busy"]);
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        out,
        "** Note: 0fs+0: hi\n    Process :busy:_p0\n** Error: 1000000fs+0: bad\n    Process :busy:_p0\n\
         ** Note: 2000000fs+0: after\n    Process :busy:_p0\n"
    );
    // `--exit-severity=error` stops at the error.
    let (code, _, out) = yavs(&["run", &lib, "--exit-severity=error", "busy"]);
    assert_eq!(code, 1);
    assert!(!out.contains("after"), "{out}");
    let (code, _, out) = yavs(&["run", &lib, "fin", "--threads=4"]);
    assert_eq!(code, 3);
    assert!(out.contains("FINISH called with status 3"), "{out}");
    // `finish` ends the calling process at once (#4).
    assert!(!out.contains("unreachable"), "{out}");
    let (code, _, _) = yavs(&["run", &lib, "fin", "--stop-time=2ns"]);
    assert_eq!(code, 0);
    // `--time-report` adds one stderr line per phase and a total, and changes nothing else.
    let (code, err, out) = yavs(&["run", &lib, "busy", "--time-report"]);
    assert_eq!(code, 1, "{err}");
    assert!(out.contains("** Note: 0fs+0: hi"), "{out}");
    for p in ["analyze", "elaborate", "init", "simulate", "total"] {
        assert!(err.contains(&format!("time-report: {p} wall=")), "{err}");
    }
    assert!(!yavs(&["run", &lib, "busy"]).1.contains("time-report"));
    // `--tier=llvm` gives the same output as the interpreter, or E0120 without the feature.
    let tier = yavs(&["run", &lib, "busy", "--tier=llvm"]);
    if cfg!(feature = "llvm") {
        assert_eq!(tier, yavs(&["run", &lib, "busy", "--tier=interp"]));
        let (_, err, _) = yavs(&["run", &lib, "busy", "--tier=llvm", "--verbose"]);
        assert!(err.contains("note: --tier=llvm: "), "{err}");
    } else {
        assert_eq!(tier.0, 2);
        assert!(tier.1.contains("error[E0120]"), "{}", tier.1);
    }
    // `--tier=auto` too, at any size cap and thread count (compilation in the background).
    let interp = yavs(&["run", &lib, "busy", "--tier=interp"]);
    for extra in [
        ["--tier-max-size=0", "--threads=1"],
        ["--tier-max-size=50000", "--threads=4"],
    ] {
        let auto = yavs(&["run", &lib, "busy", "--tier=auto", extra[0], extra[1]]);
        // Without LLVM, `auto` is the Cranelift policy when that is built in.
        if cfg!(any(feature = "llvm", feature = "clift")) {
            assert_eq!(auto, interp);
        } else {
            assert_eq!(auto.0, 2);
            assert!(auto.1.contains("`--tier=auto` needs"), "{}", auto.1);
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The exit status follows the std.env assert counts: `ClearVhdlAssert` after an error report
/// gives exit 0 (#191).
#[test]
fn run_exit_after_clear_vhdl_assert() {
    let tmp = tempdir("clear");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "use std.env.all; entity t is generic (CLEAR : boolean := true); end;\n\
         architecture a of t is begin process begin report \"bad\" severity error;\n\
           if CLEAR then ClearVhdlAssert; end if; wait; end process; end;\n",
    )
    .expect("write");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let o = run(&["analyze", "--std=2019", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = run(&["run", &lib, "t"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("** Error: 0fs+0: bad"));
    let o = run(&["run", &lib, "t", "-gCLEAR=false"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// An explicit `--exit-severity=failure` makes only failure reports count for the exit status
/// (#498); without the option an `error` report still fails the run.
#[test]
fn run_explicit_exit_severity() {
    let tmp = tempdir("exitsev");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "entity t is end;\n\
         architecture a of t is begin process begin report \"bad\" severity error; wait; end process; end;\n",
    )
    .expect("write");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let o = run(&["analyze", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for (args, code) in [
        (&["run", &lib, "t"][..], 1),
        (&["run", &lib, "--exit-severity=failure", "t"][..], 0),
        (&["run", &lib, "--exit-severity=error", "t"][..], 1),
    ] {
        let o = run(args);
        assert_eq!(o.status.code(), Some(code), "{args:?}");
        assert!(String::from_utf8_lossy(&o.stdout).contains("** Error: 0fs+0: bad"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A function called during elaboration reads a generic that has a default and an override
/// (#494, #499); an empty override is an error, not the default.
#[test]
fn run_function_reads_generic_override() {
    let tmp = tempdir("genfn");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "entity t is generic (flag : boolean := false); end;\n\
         architecture a of t is\n\
           function pick return natural is begin if flag then return 7; end if; return 3; end;\n\
           constant c : natural := pick;\n\
           signal s : bit_vector(1 to c);\n\
         begin process begin report \"len=\" & integer'image(s'length); wait; end process; end;\n",
    )
    .expect("write");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let o = run(&["analyze", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for (g, len) in [
        ("", "len=3"),
        ("-gflag=true", "len=7"),
        ("-gflag=false", "len=3"),
    ] {
        let args: Vec<&str> = ["run", &lib, "t"]
            .into_iter()
            .chain((!g.is_empty()).then_some(g))
            .collect();
        let o = run(&args);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(String::from_utf8_lossy(&o.stdout).contains(len), "{g}");
    }
    let o = run(&["run", &lib, "t", "-gflag="]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error[E2104]"));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `--resolution` sets `std.env.resolution_limit`; the default is one femtosecond, an unknown
/// unit is a usage error (#253).
#[test]
fn run_resolution() {
    let tmp = tempdir("resolution");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "use std.env.all; entity t is end;\n\
         architecture a of t is begin process begin\n\
           report integer'image(time'pos(resolution_limit)); wait; end process; end;\n",
    )
    .expect("write");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let o = run(&["analyze", "--std=2019", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for (arg, want) in [
        (None, "1"),
        (Some("--resolution=fs"), "1"),
        (Some("--resolution=ps"), "1000"),
        (Some("--resolution=ns"), "1000000"),
    ] {
        let mut a = vec!["run", &lib, "t"];
        a.extend(arg);
        let o = run(&a);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(out.contains(&format!("0fs+0: {want}\n")), "{arg:?}: {out}");
    }
    let o = run(&["run", &lib, "t", "--resolution=xs"]);
    assert_eq!(o.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// LRM 5.2.4.2: a TIME literal whose unit is below the `--resolution` limit is an error
/// anywhere in the hierarchy (a package, code that never runs), unless its abstract literal is
/// zero; the rule is about the unit, not the value (E2120).
#[test]
fn run_resolution_rejects_finer_units() {
    let tmp = tempdir("resolution-units");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    // `1 ps` sits in a function nothing calls.
    for (name, lit, ns_ok) in [
        ("fine", "1 ps", false),
        ("zero", "0 ps", true),
        ("frac", "0.4 ns", true),
        ("bare", "ps", false),
    ] {
        let src = tmp.join(format!("{name}.vhd"));
        std::fs::write(
            &src,
            format!(
                "package p is function f return time; end package;\n\
                 package body p is function f return time is begin return {lit}; end function; end package body;\n\
                 use work.p.all; entity {name} is end;\n\
                 architecture a of {name} is begin process begin wait; end process; end;\n"
            ),
        )
        .expect("write");
        let o = run(&["analyze", "--std=2008", &lib, &src.display().to_string()]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let ok = run(&["run", &lib, name, "--resolution=ps"]);
        assert!(
            ok.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
        let o = run(&["run", &lib, name, "--resolution=ns"]);
        let err = String::from_utf8_lossy(&o.stderr);
        if ns_ok {
            assert!(o.status.success(), "{name}: {err}");
        } else {
            assert_eq!(o.status.code(), Some(2), "{name}: {err}");
            assert!(err.contains("E2120"), "{name}: {err}");
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// One `yavs analyze` process per file, as VUnit runs it (`tools/corpus/run.py`): units of
/// earlier calls load from the manifests of every mapped library, including the work library,
/// whatever the case of their names; a failed file is not recorded, so its dependents fail
/// with E1002 in a later process.
#[test]
fn per_file_processes_across_libraries() {
    let tmp = tempdir("perfile");
    let src = |name: &str, text: &str| {
        let f = tmp.join(name);
        std::fs::write(&f, text).expect("write");
        f.display().to_string()
    };
    let files = [
        (
            "Lib_A",
            src(
                "a_pkg.vhd",
                "package A_Pkg is\n  function f(x : integer) return integer;\nend package;\n",
            ),
        ),
        (
            "lib_a",
            src(
                "a_body.vhd",
                "package body a_pkg is\n  function f(x : integer) return integer is\n  begin\n    return x + 1;\n  end function;\nend package body;\n",
            ),
        ),
        (
            "LIB_A",
            src(
                "a_ent.vhd",
                "library ieee;\nuse ieee.std_logic_1164.all;\nentity Ent is\n  port (o : out std_logic);\nend entity;\narchitecture Rtl of ent is\nbegin\n  o <= '1';\nend architecture;\n",
            ),
        ),
        (
            "lib_b",
            src(
                "b_ctx.vhd",
                "context B_Ctx is\n  library lib_a;\n  use lib_a.a_pkg.all;\nend context;\n",
            ),
        ),
        (
            "lib_b",
            src(
                "b_bad.vhd",
                "package bad_pkg is\n  constant c : integer := undeclared;\nend package;\n",
            ),
        ),
    ];
    let maps: Vec<String> = ["lib_a", "lib_b"]
        .iter()
        .map(|l| format!("--map={l}:{}/{l}", tmp.display()))
        .collect();
    let yavs = |lib: &str, file: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never", "analyze"])
            .arg(format!(
                "--work={lib}:{}/{}",
                tmp.display(),
                lib.to_lowercase()
            ))
            .args(&maps)
            .arg(file)
            .output()
            .expect("run");
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    for (lib, f) in &files[..4] {
        let (code, err) = yavs(lib, f);
        assert_eq!(code, Some(0), "{f}: {err}");
    }
    let (code, err) = yavs("lib_b", &files[4].1);
    assert_eq!(code, Some(1), "{err}");

    // A dependent in a new process: a context, a package and its body, an entity and its
    // architecture, all from manifests.
    let top = src(
        "top.vhd",
        "library lib_b;\ncontext lib_b.b_ctx;\nlibrary ieee;\nuse ieee.std_logic_1164.all;\nentity top is\nend entity;\narchitecture sim of top is\n  signal s : std_logic;\n  constant k : integer := f(1);\nbegin\n  u : entity lib_a.ent(rtl) port map (o => s);\nend architecture;\n",
    );
    let (code, err) = yavs("work", &top);
    assert_eq!(code, Some(0), "{err}");

    let uses_bad = src(
        "uses_bad.vhd",
        "library lib_b;\nuse lib_b.bad_pkg.all;\nentity uses_bad is\nend entity;\n",
    );
    let (code, err) = yavs("work", &uses_bad);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("error[E1002]"), "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn explain() {
    let yavs = |code: &str| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--explain", code])
            .output()
            .expect("run yavs")
    };
    let ok = yavs("e3002");
    assert!(ok.status.success());
    assert!(
        String::from_utf8(ok.stdout)
            .unwrap()
            .starts_with("E3002: More delta cycles")
    );
    let sem = yavs("E1100");
    assert!(sem.status.success());
    assert_eq!(yavs("E0000").status.code(), Some(2));
}

/// An empty or missing work directory is a valid empty library (cli-contract section 3): `run`
/// and `elab` report the top level as not found (`E0104`) and name the directory, not `L0001`;
/// a wrong top level gets no "unknown configuration" and a library prefix is explained (#838).
#[test]
fn top_level_not_found() {
    let tmp = tempdir("notop");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "entity good is end;\narchitecture a of good is begin end;\n",
    )
    .unwrap();
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        (
            o.status.code().expect("code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let missing = format!("--work=work:{}/none", tmp.display());
    let empty = tmp.join("empty");
    std::fs::create_dir(&empty).unwrap();
    let empty = format!("--work=work:{}", empty.display());
    for cmd in ["run", "elab"] {
        let (code, err) = yavs(&[cmd, &missing, "good"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("error[E0104]: top level `good` not found in library `work`"),
            "{err}"
        );
        assert!(err.contains("none` does not exist"), "{err}");
        assert!(!err.contains("L0001"), "{err}");
        let (code, err) = yavs(&[cmd, &empty, "good"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("error[E0104]"), "{err}");
        assert!(err.contains("has no analyzed files"), "{err}");
    }
    assert!(
        !tmp.join("none").exists(),
        "run and elab never create the library"
    );
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert_eq!(yavs(&["analyze", &lib, &src.display().to_string()]).0, 0);
    let (code, err) = yavs(&["run", &lib, "nosuch"]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("error[E0104]: top level `nosuch` not found"),
        "{err}"
    );
    assert!(!err.contains("unknown configuration"), "{err}");
    let (code, err) = yavs(&["run", &lib, "work.good"]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("TOP has no library prefix: write `good`"),
        "{err}"
    );
    assert_eq!(yavs(&["run", &lib, "good.a"]).0, 0);
}

/// `--message-format=json` prints nothing but JSON objects, one per line, also when `analyze`
/// stops at one file of several (#839); `--threads=0` and a unit-less `--stop-time` are usage
/// errors that say what to write.
#[test]
fn json_stream_and_numeric_options() {
    let tmp = tempdir("jsonstream");
    std::fs::write(
        tmp.join("ok.vhd"),
        "entity ok is end;\narchitecture a of ok is begin end;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("bad.vhd"),
        "entity bad is end;\narchitecture a of bad is begin x <= 1; end;\n",
    )
    .unwrap();
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .current_dir(&*tmp)
            .output()
            .expect("run");
        (
            o.status.code().expect("code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let lib = format!("--work=work:{}/lib", tmp.display());
    let (code, err) = yavs(&[
        "--message-format=json",
        "analyze",
        &lib,
        "ok.vhd",
        "bad.vhd",
    ]);
    assert_eq!(code, 1, "{err}");
    let lines: Vec<&str> = err.lines().collect();
    assert!(lines.len() >= 2, "{err}");
    for l in &lines {
        let v: serde_json::Value =
            serde_json::from_str(l).unwrap_or_else(|e| panic!("`{l}`: {e}\n{err}"));
        assert!(v["level"].is_string() && v["message"].is_string(), "{l}");
    }
    assert!(
        lines
            .last()
            .unwrap()
            .contains("analysis stopped at `bad.vhd`"),
        "{err}"
    );
    assert!(lines.last().unwrap().contains("ok.vhd"), "{err}");
    // The numeric options of `run`.
    assert_eq!(yavs(&["analyze", &lib, "ok.vhd"]).0, 0);
    let (code, err) = yavs(&["run", &lib, "ok", "--threads=0"]);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("the number of threads must be at least 1"),
        "{err}"
    );
    let (code, err) = yavs(&["run", &lib, "ok", "--threads=2"]);
    assert_eq!(code, 0, "{err}");
    let (code, err) = yavs(&["run", &lib, "ok", "--stop-time=10"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("a time needs a unit"), "{err}");
    assert!(err.contains("--stop-time=10ns"), "{err}");
    assert!(!err.contains("invalid unit value"), "{err}");
}

/// `--ieee-warnings=off` drops the warnings raised inside `ieee` packages, from the native
/// subprograms and from the VHDL bodies alike, and nothing else; any other value is a usage
/// error (#836).
#[test]
fn ieee_warnings_option() {
    let tmp = tempdir("ieeew");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;\n\
         entity t is end;\n\
         architecture a of t is signal u : unsigned(3 downto 0) := \"01X1\"; begin\n\
           process variable i : integer; begin\n\
             i := to_integer(u);\n\
             i := to_integer(unsigned'(\"1X\"));\n\
             report \"mine\" severity warning;\n\
             wait; end process; end;\n",
    )
    .unwrap();
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str], bodies: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.args(["--color", "never"]).args(args);
        if bodies {
            c.env("YAVS_NO_NATIVE_IEEE", "1");
        }
        let o = c.output().expect("run");
        (
            o.status.code().expect("code"),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    assert_eq!(
        yavs(&["analyze", &lib, &src.display().to_string()], false).0,
        0
    );
    for bodies in [false, true] {
        for flag in [None, Some("--ieee-warnings=on")] {
            let mut args = vec!["run", &lib, "t"];
            args.extend(flag);
            let (code, out, err) = yavs(&args, bodies);
            assert_eq!(code, 0, "{err}");
            assert_eq!(
                out.matches("NUMERIC_STD.TO_INTEGER: metavalue detected")
                    .count(),
                2,
                "{out}"
            );
            assert!(out.contains("mine"), "{out}");
        }
        let (code, out, err) = yavs(&["run", &lib, "t", "--ieee-warnings=off"], bodies);
        assert_eq!(code, 0, "{err}");
        assert!(!out.contains("NUMERIC_STD"), "{out}");
        assert!(out.contains("** Warning: 0fs+0: mine"), "{out}");
    }
    let (code, _, err) = yavs(&["run", &lib, "t", "--ieee-warnings=maybe"], false);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("invalid value 'maybe'"), "{err}");
}

/// `-D NAME=VALUE` is accepted by `elab` and `run` as by `analyze`: the sources are read again
/// there, so a file that needs a conditional analysis identifier needs it at every step (#837).
#[test]
fn defines_reach_elab_and_run() {
    let tmp = tempdir("defines");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "entity t is end;\narchitecture a of t is begin process begin\n\
         `if FOO = \"1\" then\nreport \"foo one\";\n`else\nreport \"foo other\";\n`end if\n\
         wait; end process; end;\n",
    )
    .unwrap();
    let lib = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        (
            o.status.code().expect("code"),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let file = src.display().to_string();
    let (code, _, err) = yavs(&["analyze", &lib, "-D", "FOO=1", &file]);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = yavs(&["run", &lib, "t"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("error[E0007]"), "{err}");
    let (code, out, err) = yavs(&["run", &lib, "-D", "FOO=1", "t"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("foo one"), "{out}");
    let (code, out, err) = yavs(&["run", &lib, "--define=FOO=2", "t"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("foo other"), "{out}");
    let (code, _, err) = yavs(&["elab", &lib, "-DFOO=1", "t"]);
    assert_eq!(code, 0, "{err}");
}

/// The help and the explanation of `E0120` describe the tiers this build has and point at no
/// developer document (#845, #850).
#[test]
fn tier_help_and_e0120_describe_the_build() {
    let text = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(args)
            .output()
            .expect("run yavs");
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let has = if cfg!(all(feature = "llvm", feature = "clift")) {
        "the Cranelift and LLVM tiers"
    } else if cfg!(feature = "clift") {
        "the Cranelift tier"
    } else if cfg!(feature = "llvm") {
        "the LLVM tier"
    } else {
        "no compiled tier, only `interp`"
    };
    let help = text(&["run", "--help"]);
    assert!(help.contains(&format!("this yavs has {has}")), "{help}");
    let e0120 = text(&["--explain", "E0120"]);
    assert!(e0120.contains("clift"), "{e0120}");
    assert!(e0120.contains(&format!("This yavs has {has}.")), "{e0120}");
    // No help names a developer document.
    for cmd in ["analyze", "elab", "run", "parse", "cache", "cov"] {
        let help = text(&[cmd, "--help"]);
        for internal in ["sem.md", "cli-contract", "docs/", "AGENTS"] {
            assert!(
                !help.contains(internal),
                "`{cmd} --help` names {internal}:\n{help}"
            );
        }
    }
    assert!(!e0120.contains("docs/"), "{e0120}");
}

fn etxtbsy(e: &std::io::Error) -> bool {
    cfg!(unix) && e.raw_os_error() == Some(26)
}

/// A copy of the binary alone in a directory analyzes and runs a design (std/ieee are built
/// in). `$YAVS_STDLIB` naming a tree with empty `std` and `ieee` must fail and name that tree,
/// which shows the variable replaces the built-in copy.
#[test]
fn standalone_binary() {
    // std/ieee are built into the binary: a copy alone in a directory runs a design.
    let exe = Path::new(env!("CARGO_BIN_EXE_yavs"));
    let tmp = tempdir("installed");
    let src = tmp.join("t.vhd");
    std::fs::write(
        &src,
        "library ieee; use ieee.std_logic_1164.all;\n\
         entity t is end;\n\
         architecture a of t is signal s : std_logic := '0'; begin\n\
         process begin s <= '1'; wait for 1 ns; report \"s=\" & std_logic'image(s); wait; end process; end;\n",
    )
    .expect("write");
    let work = format!("--work=work:{}", tmp.join("work").display());
    let yavs = |bin: &Path, stdlib: Option<&Path>, args: &[&str]| {
        let mut c = Command::new(bin);
        match stdlib {
            Some(d) => c.env("YAVS_STDLIB", d),
            None => c.env_remove("YAVS_STDLIB"),
        };
        c.args(["--color", "never"]).args(args);
        // A sibling test forking while we wrote the copy leaves the copy's write fd open in the
        // child until its exec, so exec of the copy can fail with ETXTBSY (Linux). Retry that only.
        let mut tries = 0;
        let o = loop {
            match c.output() {
                Err(e) if etxtbsy(&e) && tries < 100 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                r => break r.expect("run"),
            }
        };
        (
            o.status.code().expect("exit code"),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let bin = tmp.join("inst").join(exe.file_name().expect("exe name"));
    std::fs::create_dir_all(tmp.join("inst")).expect("mkdir");
    std::fs::copy(exe, &bin).expect("copy yavs");
    let s = src.display().to_string();
    let (code, err, _) = yavs(&bin, None, &["analyze", &work, &s]);
    assert_eq!(code, 0, "{err}");
    let (code, err, out) = yavs(&bin, None, &["run", &work, "t"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("s='1'"), "{out}");

    // `$YAVS_STDLIB` replaces the built-in tree, and a missing unit names where yavs looked.
    let empty = tmp.join("empty");
    for d in ["std", "ieee"] {
        std::fs::create_dir_all(empty.join(d)).expect("mkdir");
    }
    let work = format!("--work=work:{}", tmp.join("work2").display());
    let (code, err, _) = yavs(&bin, Some(&empty), &["analyze", &work, &s]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("error[E1002]"), "{err}");
    let looked = empty.join("std");
    assert!(err.contains(&looked.display().to_string()), "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The same source file analyzed into two libraries gives each library its own units, and a
/// design using both elaborates (#290).
#[test]
fn same_file_in_two_libraries() {
    let tmp = tempdir("two_libs");
    let write = |name: &str, text: &str| std::fs::write(tmp.join(name), text).expect("write");
    write(
        "shared.vhd",
        "package shared_pkg is constant C : integer := 1; end package;\n",
    );
    write(
        "user.vhd",
        "use work.shared_pkg.all;\npackage user_pkg is constant D : integer := C + 1; end package;\n",
    );
    write(
        "top.vhd",
        "library liba, libb;\nuse liba.user_pkg.all;\nuse libb.user_pkg.all;\n\
         entity top is end entity;\n\
         architecture a of top is begin process begin report \"ok\"; wait; end process; end;\n",
    );
    let dir = |l: &str| format!("{}/{l}", tmp.display());
    let maps = [
        format!("--map=liba:{}", dir("liba")),
        format!("--map=libb:{}", dir("libb")),
    ];
    let run = |args: &[String]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let analyze = |lib: &str, file: &str| {
        let mut a = vec!["analyze".to_string(), format!("--work={lib}:{}", dir(lib))];
        a.extend(maps.iter().cloned());
        a.push(tmp.join(file).display().to_string());
        let o = run(&a);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    for lib in ["liba", "libb"] {
        analyze(lib, "shared.vhd");
        analyze(lib, "user.vhd");
    }
    analyze("libc", "top.vhd");
    let mut a = vec!["run".to_string(), format!("--work=libc:{}", dir("libc"))];
    a.extend(maps.iter().cloned());
    a.push("top".into());
    let o = run(&a);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("ok"));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `run --gui` starts `yavs-gui` with `--yavs <this yavs> -- <run arguments>` and exits with its
/// status; without one next to yavs or on PATH it is E0122, exit 2.
#[test]
fn run_gui() {
    let dir = tempdir("gui");
    let yavs = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.env("PATH", &*dir).env("NO_COLOR", "1");
        c.args(["run", "--gui", "--work=work:w", "tb", "--stop-time=1ns"]);
        c
    };
    let o = yavs().output().expect("yavs");
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("error[E0122]: `yavs-gui` was not found"),
        "{err}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let gui = dir.join("yavs-gui");
        let log = dir.join("args");
        std::fs::write(
            &gui,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexit 7\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&gui, std::fs::Permissions::from_mode(0o755)).unwrap();
        let o = yavs().output().expect("yavs");
        assert_eq!(o.status.code(), Some(7));
        let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_yavs")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!(
                "--yavs\n{}\n--\n--work=work:w\ntb\n--stop-time=1ns\n",
                exe.display()
            )
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `yavs` with colours off.
fn yavs_cmd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run")
}

/// Analyzes `files` (relative to `dir`) into library `lib` mapped at `dir/lib`, others by `maps`.
fn analyze_into(dir: &std::path::Path, lib: &str, maps: &[&str], files: &[(&str, &str)]) {
    let mut args = vec![
        "analyze".to_string(),
        "--std=2008".to_string(),
        format!("--work={lib}:{}/{lib}", dir.display()),
    ];
    args.extend(
        maps.iter()
            .map(|m| format!("--map={m}:{}/{m}", dir.display())),
    );
    for (name, text) in files {
        std::fs::write(dir.join(name), text).expect("write");
        args.push(dir.join(name).display().to_string());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let o = yavs_cmd(&args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// An elaboration-time call of a package function from an entity that is loaded while the design
/// is elaborated (a default-bound component analyzed after its user): the package comes with
/// the entity and its body must be loaded too (#515, #516).
#[test]
fn run_package_body_of_a_late_loaded_entity() {
    let tmp = tempdir("latebody");
    let dir: &std::path::Path = &tmp;
    analyze_into(
        dir,
        "lib2",
        &[],
        &[
            (
                "pk.vhd",
                "package pk is function twice(x : natural) return natural; end package;\n\
                 package body pk is\n\
                   function twice(x : natural) return natural is begin return 2 * x; end function;\n\
                 end package body;\n",
            ),
            (
                "comps.vhd",
                "package comps is component leaf generic (n : natural := 3); end component; end package;\n",
            ),
            (
                "top.vhd",
                "use work.comps.all;\n\
                 entity top is end entity;\n\
                 architecture a of top is begin u : leaf generic map (n => 5); end architecture;\n",
            ),
        ],
    );
    analyze_into(
        dir,
        "lib2",
        &[],
        &[(
            "leaf.vhd",
            "use work.pk.all;\n\
             entity leaf is generic (n : natural := 3); end entity;\n\
             architecture a of leaf is\n\
               impure function init_value return natural is begin return twice(n); end function;\n\
               signal s : natural := init_value;\n\
             begin process begin report \"s = \" & natural'image(s); wait; end process; end architecture;\n",
        )],
    );
    let lib = format!("--work=lib2:{}/lib2", dir.display());
    let o = yavs_cmd(&["run", "--std=2008", &lib, "top"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("s = 10"));
}

/// Packages are initialised in dependency order, not in the order their units happened to be
/// loaded: the body of `ap` names `tp` (another library) and its initialisation calls into it
/// (#517).
#[test]
fn run_package_initialisation_follows_dependencies() {
    let tmp = tempdir("pkgorder");
    let dir: &std::path::Path = &tmp;
    analyze_into(
        dir,
        "libt",
        &[],
        &[(
            "t.vhd",
            "package tp is procedure say(msg : string); end package;\n\
             package body tp is\n\
               type flag_t is protected\n\
                 procedure set(v : boolean);\n\
                 impure function get return boolean;\n\
               end protected;\n\
               type flag_t is protected body\n\
                 variable f : boolean := false;\n\
                 procedure set(v : boolean) is begin f := v; end procedure;\n\
                 impure function get return boolean is begin return f; end function;\n\
               end protected body;\n\
               shared variable enabled : flag_t;\n\
               procedure say(msg : string) is begin if not enabled.get then report msg; end if; end procedure;\n\
             end package body;\n",
        )],
    );
    analyze_into(
        dir,
        "liba",
        &["libt"],
        &[
            (
                "a.vhd",
                "package ap is impure function loud(x : natural) return natural; end package;\n",
            ),
            (
                "b.vhd",
                "library libt;\n\
                 use libt.tp.all;\n\
                 package body ap is\n\
                   type cnt_t is protected impure function next_id return natural; end protected;\n\
                   type cnt_t is protected body\n\
                     variable n : natural := 0;\n\
                     impure function next_id return natural is begin say(\"next_id\"); n := n + 1; return n; end function;\n\
                   end protected body;\n\
                   shared variable ids : cnt_t;\n\
                   impure function loud(x : natural) return natural is\n\
                   begin say(\"loud\"); return x + ids.next_id; end function;\n\
                   constant first_id : natural := loud(1);\n\
                 end package body;\n",
            ),
        ],
    );
    analyze_into(
        dir,
        "work",
        &["liba", "libt"],
        &[(
            "top.vhd",
            "library liba;\n\
             use liba.ap.all;\n\
             entity top is generic (g : natural := loud(2)); end entity;\n\
             architecture a of top is begin\n\
               process begin report \"g=\" & natural'image(g); wait; end process;\n\
             end architecture;\n",
        )],
    );
    let w = format!("--work=work:{}/work", dir.display());
    let (a, t) = (
        format!("--map=liba:{}/liba", dir.display()),
        format!("--map=libt:{}/libt", dir.display()),
    );
    let o = yavs_cmd(&["run", "--std=2008", &w, &a, &t, "top"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("g=4"));
}

/// A 2019 `T'VALUE` of an array type elaborates and evaluates (the first reference
/// suite's `value4`, #609).
#[test]
fn composite_value_attribute() {
    let dir = tempdir("composite_value");
    let lib = format!("--work=work:{}/lib", dir.display());
    let src = dir.join("top.vhd");
    std::fs::write(
        &src,
        "entity top is end entity;\n\
         architecture a of top is\n\
           type abc is ('a', 'b', 'c');\n\
           type abc_vec is array (natural range <>) of abc;\n\
         begin\n\
           process begin\n\
             assert abc_vec'value(\"abc\") = \"abc\";\n\
             wait;\n\
           end process;\n\
         end architecture;\n",
    )
    .expect("write");
    let o = yavs_cmd(&["analyze", "--std=2019", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = yavs_cmd(&["run", "--std=2019", &lib, "top"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// An aggregate target made of fields of a signal parameter is not lowered yet: an explicit
/// E2001, not an internal error (the second reference suite's `issue3030`).
#[test]
fn aggregate_target_of_signal_parameter_is_not_an_ice() {
    let dir = tempdir("agg_target_param");
    let lib = format!("--work=work:{}/lib", dir.display());
    let src = dir.join("top.vhd");
    std::fs::write(
        &src,
        "entity top is end entity;\n\
         architecture a of top is\n\
           type rec_t is record x : bit_vector(0 to 1); y : bit_vector(0 to 1); end record;\n\
           procedure split (signal bv : in bit_vector; signal r : out rec_t) is\n\
           begin\n\
             (r.x, r.y) <= bv;\n\
           end procedure;\n\
           signal s : rec_t;\n\
           signal b : bit_vector(0 to 3) := \"1001\";\n\
         begin\n\
           process begin split(b, s); wait; end process;\n\
         end architecture;\n",
    )
    .expect("write");
    let o = yavs_cmd(&["analyze", "--std=2008", &lib, &src.display().to_string()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = yavs_cmd(&["elab", "--std=2008", &lib, "top"]);
    let _ = std::fs::remove_dir_all(&dir);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("E2001"), "{err}");
    assert!(!err.contains("E9999"), "{err}");
}
