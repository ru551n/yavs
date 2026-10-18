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
        .replace(t, "$TMP");
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
