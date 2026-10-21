//! Snapshot tests for `yavs analyze` (exit code and stderr), in the style of diag.rs.
//!
//! Cases are listed below; sources are in `tests/cli`, expected output in
//! `tests/cli/NAME.stderr` with the scratch directory shown as `$TMP`.
//! `YAVS_BLESS=1` rewrites the expected files.

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
        args: &["analyze", "--std=93", "--work=work:$TMP/lib", "ok.vhd"],
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
        let t = std::time::Duration::from_secs(3);
        let Ok(first) = rx.recv_timeout(t) else {
            child.kill().expect("kill");
            panic!("{threads}: no report in 3 s");
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

fn tempdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("yavs-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
    d
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
    assert!(
        m.contains("name = \"ok\"") && m.contains("std = \"2008\""),
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
                m.contains(&format!("name = \"{p}{i}\"")),
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
           begin return (a * 31 + b) mod 1000003; end function;\n\
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
    assert!(err.contains("error[L0031]"), "{err}");
    // A design without processes runs to event starvation.
    let (code, err, _) = yavs(&["run", &lib, "--exit-severity=error", "top"]);
    assert_eq!(code, 0, "{err}");
    // Reports in NVC's format on stdout. By default an error report does not stop the run,
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
        if cfg!(feature = "llvm") {
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
        c.env("PATH", &dir).env("NO_COLOR", "1");
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
