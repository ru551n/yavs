//! `yavs run --profile`: the tables name the hot process and its line (`tests/cli/profile.vhd`),
//! the design's output is unchanged, and the unsupported and failing cases fail clearly.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const HOT_LINE: u64 = 16;
/// The statement in `burn`'s loop, which the optimiser inlines into `hot`.
const BURN_LINE: u64 = 10;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run")
}

fn setup(name: &str) -> common::Tmp {
    let t = tempdir(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/profile.vhd");
    std::fs::copy(src, t.join("profile.vhd")).expect("copy");
    let a = yavs(&t, &["analyze", "--work=work:lib", "profile.vhd"]);
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    t
}

fn run(t: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["run", "--work=work:lib", "profile"];
    args.extend_from_slice(extra);
    yavs(t, &args)
}

#[test]
fn names_the_hot_process_and_line() {
    let t = setup("profile");
    let plain = run(&t, &[]);
    assert!(plain.status.success());
    for extra in [
        &["--profile=p.json"][..],
        &["--profile=p.json", "--tier=interp"],
        &["--profile=p.json", "--threads=2"],
    ] {
        let o = run(&t, extra);
        assert!(o.status.success(), "{extra:?}");
        // Profiling changes nothing the design prints.
        assert_eq!(o.stdout, plain.stdout, "{extra:?}");
        let err = String::from_utf8_lossy(&o.stderr);
        let first = err
            .lines()
            .skip_while(|l| !l.starts_with("profile: processes"))
            .nth(2)
            .unwrap_or_else(|| panic!("no process row:\n{err}"));
        assert!(first.contains(":profile:hot"), "{first}\n{err}");
        assert!(
            first.contains(&format!("profile.vhd:{HOT_LINE}")),
            "{first}"
        );
        let j: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(t.join("p.json")).expect("file"))
                .expect("json");
        let p = &j["processes"][0];
        assert_eq!(p["path"], ":profile:hot");
        assert_eq!(p["line"], HOT_LINE);
        assert!(
            p["file"]
                .as_str()
                .is_some_and(|f| f.ends_with("profile.vhd"))
        );
        assert!(j["samples"].as_u64() > Some(0));
        assert!(p["samples"].as_u64() > j["processes"][1]["samples"].as_u64());
        assert_eq!(j["instances"][0]["path"], ":profile");
        // The interpreter names the statement it spends its time in (compiled code has none).
        if extra.contains(&"--tier=interp") {
            let l = &j["lines"][0];
            assert_eq!(l["line"], BURN_LINE, "{j}");
            assert_eq!(l["subprogram"], "burn");
            assert!(
                err.contains(&format!("profile.vhd:{BURN_LINE} (burn)")),
                "{err}"
            );
        }
    }
}

#[test]
fn without_a_file_only_the_tables_are_printed() {
    let t = setup("profile-nofile");
    let o = run(&t, &["--profile"]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("profile: subprograms"));
    assert!(!t.join("p.json").exists());
}

#[test]
fn unwritable_file_is_e0124() {
    let t = setup("profile-bad");
    let o = run(&t, &["--profile=no/such/dir/p.json"]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("E0124"), "{err}");
}

#[test]
fn not_with_dap() {
    let t = setup("profile-dap");
    let o = run(&t, &["--profile", "--dap=stdio"]);
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("E0125"), "{err}");
    assert!(!err.contains("E9999"), "{err}");
}

/// Ctrl-C prints and writes the profile of the run so far; the exit status stays 130.
#[cfg(unix)]
#[test]
fn sigint_prints_the_profile_so_far() {
    use std::io::Read;
    use std::process::Stdio;
    let t = setup("profile-sigint");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/profile_forever.vhd"),
        t.join("profile_forever.vhd"),
    )
    .expect("copy");
    let a = yavs(&t, &["analyze", "--work=work:lib", "profile_forever.vhd"]);
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    // No stop time: the design runs until interrupted.
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "profile_forever",
        ])
        .arg("--profile=p.json")
        .current_dir(&t)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let pid = c.id().to_string();
    let k = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("kill");
    assert!(k.success());
    let mut err = String::new();
    c.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    let st = c.wait().unwrap();
    assert_eq!(st.code(), Some(130), "{err}");
    assert!(err.contains("profile: processes"), "{err}");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(t.join("p.json")).expect("file"))
            .expect("json");
    assert!(j["samples"].as_u64() > Some(0));
}
