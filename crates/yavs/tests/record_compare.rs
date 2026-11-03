//! A whole record drive compared with the record's image (`gather.rs`, `docs/decisions.md` "Record
//! drive compare") changes nothing. Each design runs without images (the kernel decides every
//! leaf), with the compare after its warm-up, and from the first scatter with every skip held
//! against the kernel's full test, always in debug builds. Output and waves must be the same, and
//! the compare must fire: `scatter_spared` has a floor.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const OFF: &[(&str, &str)] = &[("YAVS_NO_IMAGE", "1")];
const CHECK: &[(&str, &str)] = &[("YAVS_IMAGE_AFTER", "1"), ("YAVS_IMAGE_CHECK", "1")];

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn spared(err: &str) -> u64 {
    let key = "counters: scatter_spared=";
    err.lines()
        .find_map(|l| l.strip_prefix(key)?.parse().ok())
        .unwrap_or_else(|| panic!("no {key} in {err}"))
}

/// Analysed `file`, its library argument, and runs of `top` as (status, stdout, leaves spared).
struct Design {
    dir: common::Tmp,
    work: String,
    top: String,
}

impl Design {
    fn new(tag: &str, file: &Path, top: &str) -> Design {
        let dir = tempdir(tag);
        let work = format!("work:{}", dir.join("lib").display());
        let a = yavs(
            &dir,
            OFF,
            &[
                "analyze",
                "--std=2008",
                "--work",
                &work,
                file.to_str().unwrap(),
            ],
        );
        assert!(a.status.success(), "analyze {tag}: {a:?}");
        Design {
            dir,
            work,
            top: top.to_string(),
        }
    }

    fn run(&self, env: &[(&str, &str)], extra: &[&str]) -> (Option<i32>, String, u64) {
        let mut args = vec!["run", "--std=2008", "--work", &self.work];
        args.extend(extra);
        args.push(&self.top);
        let mut e = vec![("YAVS_COUNTERS", "1")];
        e.extend(env);
        let r = yavs(&self.dir, &e, &args);
        let err = String::from_utf8_lossy(&r.stderr);
        (
            r.status.code(),
            String::from_utf8_lossy(&r.stdout).into_owned(),
            spared(&err),
        )
    }
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/corpus/held/{name}.vhd"))
}

/// Force, release, entries and waveforms of the run, delays, parts, resolved records, ports, a
/// procedure's drive, the deltas of a run that only holds: `held/held_record`.
#[test]
fn the_compare_changes_nothing_around_what_must_see_a_drive() {
    let d = Design::new("record_compare", &corpus("held_record"), "held_record");
    let interp = ["--tier=interp", "--no-cache"];
    let off = d.run(OFF, &interp);
    assert_eq!(off.0, Some(0), "{}", off.1);
    assert_eq!(off.2, 0, "no images, no compare");
    let warm = d.run(&[], &interp);
    assert_eq!((&off.0, &off.1), (&warm.0, &warm.1), "the default warm-up");
    assert!(warm.2 > 10_000, "only {} leaves spared", warm.2);
    let first = d.run(CHECK, &interp);
    assert_eq!(
        (&off.0, &off.1),
        (&first.0, &first.1),
        "from the first scatter"
    );
    assert!(first.2 > warm.2, "{} against {}", first.2, warm.2);
    for threads in ["--threads=4", "--threads=4 --shuffle=11"] {
        let mut args = interp.to_vec();
        args.extend(threads.split(' '));
        let got = d.run(CHECK, &args);
        // A shuffled run reports processes of one cycle in another order: compare sorted.
        let sorted = |s: &str| {
            let mut v: Vec<_> = s.lines().map(str::to_owned).collect();
            v.sort();
            v
        };
        assert_eq!(off.0, got.0, "{threads}");
        assert_eq!(sorted(&off.1), sorted(&got.1), "{threads}");
    }
}

#[test]
fn the_compare_changes_no_wave() {
    let d = Design::new("record_compare_w", &corpus("held_record"), "held_record");
    let (a, b) = (d.dir.join("off.fst"), d.dir.join("on.fst"));
    for (env, f) in [(OFF, &a), (CHECK, &b)] {
        let wave = format!("--wave={}", f.display());
        let r = d.run(env, &["--tier=interp", "--no-cache", &wave]);
        assert_eq!(r.0, Some(0), "{}", r.1);
    }
    let on = std::fs::read(&b).unwrap();
    assert!(on.len() > 1000);
    assert_eq!(std::fs::read(&a).unwrap(), on, "waves differ");
}

/// In process copies with facts, where a field's drive is kept in the batch of compiled code or
/// compared inline before the whole record's: both compares at once, both checked.
#[cfg(feature = "clift")]
#[test]
fn the_compare_and_the_compare_in_compiled_code_work_together() {
    let d = Design::new("record_compare_c", &corpus("held_record"), "held_record");
    let interp = d.run(OFF, &["--tier=interp", "--no-cache"]);
    assert_eq!(interp.0, Some(0), "{}", interp.1);
    let env = [
        ("YAVS_IMAGE_AFTER", "1"),
        ("YAVS_IMAGE_CHECK", "1"),
        ("YAVS_INLCMP", "check"),
        ("YAVS_SPEC_ALL", "1"),
        ("YAVS_SPEC_AFTER", "2"),
    ];
    let heat = ["--tier=clift-auto", "--tier-heat=1,0,0", "--no-cache"];
    let got = d.run(&env, &heat);
    assert_eq!((&interp.0, &interp.1), (&got.0, &got.1), "compiled");
    assert!(got.2 > 10_000, "only {} leaves spared", got.2);
    let mut args = vec!["run", "--std=2008", "--work", &d.work];
    args.extend(heat);
    args.push(&d.top);
    let mut e = env.to_vec();
    e.push(("YAVS_COUNTERS", "1"));
    let r = yavs(&d.dir, &e, &args);
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("counters: gate_checked="),
        "no inline compare: {err}"
    );
    let forced = d.run(CHECK, &["--tier=clift", "--no-cache"]);
    assert_eq!(
        (&interp.0, &interp.1),
        (&forced.0, &forced.1),
        "forced compiled"
    );
    assert!(forced.2 > 10_000, "forced: only {} leaves spared", forced.2);
}
