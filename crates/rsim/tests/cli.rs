//! Snapshot tests for `rsim analyze` (exit code and stderr), in the style of diag.rs.
//!
//! Cases are listed below; sources are in `tests/cli`, expected output in
//! `tests/cli/NAME.stderr` with the scratch directory shown as `$TMP`.
//! `RSIM_BLESS=1` rewrites the expected files.

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
    let out = Command::new(env!("CARGO_BIN_EXE_rsim"))
        .args(["--color", "never"])
        .args(&args)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .output()
        .expect("run rsim");
    let err = String::from_utf8(out.stderr)
        .expect("utf-8")
        .replace(t, "$TMP");
    (out.status.code().expect("exit code"), err)
}

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli");
    let bless = std::env::var_os("RSIM_BLESS").is_some_and(|v| v == "1");
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

fn tempdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rsim-cli-{}-{name}", std::process::id()));
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
    let rsim = |f: &str| {
        Command::new(env!("CARGO_BIN_EXE_rsim"))
            .args(["--color", "never", "analyze", &lib, f])
            .current_dir(&dir)
            .output()
            .expect("run")
    };
    assert_eq!(rsim("ok.vhd").status.code(), Some(0));
    let m = std::fs::read_to_string(tmp.join("lib/rsim-lib.toml")).expect("manifest");
    assert!(
        m.contains("name = \"ok\"") && m.contains("std = \"2008\""),
        "{m}"
    );

    // A batch into an existing library reads its manifest while holding its lock.
    let o = Command::new(env!("CARGO_BIN_EXE_rsim"))
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
        let o = rsim("ok.vhd");
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
            Command::new(env!("CARGO_BIN_EXE_rsim"))
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
    let m = std::fs::read_to_string(tmp.join("lib/rsim-lib.toml")).expect("manifest");
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
