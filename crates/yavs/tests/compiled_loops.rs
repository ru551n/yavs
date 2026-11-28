//! Element loops in compiled code (#757): on a native frame the iterations of a copy or fill loop
//! but the last run planned in the code itself (or in `yavs_rt_eloop` for the shapes it leaves), and the output, the exit status and the message of a loop
//! that leaves its vector are those of the interpreter, which runs the plain loop.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

const LOOPS: &str = include_str!("loops/loops.vhd");

fn run(
    d: &std::path::Path,
    tier: &[&str],
    counters: bool,
    helper: Option<&str>,
) -> (Option<i32>, String, String) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let mut c = Command::new(yavs);
    c.current_dir(d)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(tier)
        .arg("loops");
    if counters {
        c.env("YAVS_COUNTERS", "1");
    }
    if let Some(name) = helper {
        // The loops planned by the runtime helper instead of the code itself.
        c.env(name, "1");
    }
    let o = c.output().expect("run");
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// The lines of a run that the loops make, without the counters.
fn text(r: &(Option<i32>, String, String)) -> String {
    let keep = |s: &str| {
        s.lines()
            .filter(|l| !l.starts_with("counters:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!("{:?}\n{}\n{}", r.0, keep(&r.1), keep(&r.2))
}

#[test]
fn element_loops_in_compiled_code_match_the_interpreter() {
    let d = tempdir("compiled_loops");
    std::fs::write(d.join("loops.vhd"), LOOPS).expect("write");
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "loops.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let want = run(&d, &["--tier=interp"], false, None);
    assert!(text(&want).contains("short LLLHH000"), "{}", text(&want));
    let mut tiers = Vec::new();
    if cfg!(feature = "clift") {
        let h = Some("YAVS_ELOOP_HELPER");
        tiers.push((vec!["--tier=clift"], 5, None));
        tiers.push((vec!["--tier=clift-auto", "--tier-heat=1,0,0"], 1, None));
        tiers.push((vec!["--tier=clift"], 5, h));
        tiers.push((vec!["--tier=clift-auto", "--tier-heat=1,0,0"], 1, h));
    }
    for (tier, least, helper) in tiers {
        let got = run(&d, &tier, true, helper);
        assert_eq!(text(&got), text(&want), "{tier:?} helper {helper:?}");
        // The loops ran at once for the loops of enough elements (a build with callback counters).
        let calls = got
            .1
            .lines()
            .chain(got.2.lines())
            .find_map(|l| l.strip_prefix("counters: cb_eloop="))
            .and_then(|n| n.parse::<u64>().ok());
        if let Some(n) = calls {
            assert!(n >= least, "{tier:?} helper {helper:?}: {n} calls");
        }
    }
}

/// The calls of the runtime helper a run counted (`None`: no such counter line).
fn helper_calls(r: &(Option<i32>, String, String)) -> Option<u64> {
    r.1.lines()
        .chain(r.2.lines())
        .find_map(|l| l.strip_prefix("counters: cb_eloop="))
        .and_then(|n| n.parse().ok())
}

/// `YAVS_LLVM_ELOOP_HELPER` is the old name of `YAVS_ELOOP_HELPER` and still switches the helper
/// on (accepted for one release).
#[test]
fn the_old_name_of_the_helper_switch_still_works() {
    let d = tempdir("compiled_loops_old_name");
    std::fs::write(d.join("loops.vhd"), LOOPS).expect("write");
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "loops.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let tier = ["--tier=clift"];
    let planned = run(&d, &tier, true, None);
    let new = run(&d, &tier, true, Some("YAVS_ELOOP_HELPER"));
    let old = run(&d, &tier, true, Some("YAVS_LLVM_ELOOP_HELPER"));
    assert_eq!(text(&old), text(&new));
    let (planned_n, new_n, old_n) = (
        helper_calls(&planned).unwrap_or(0),
        helper_calls(&new).unwrap_or(0),
        helper_calls(&old).unwrap_or(0),
    );
    // (`cb_eloop` is counted in debug and `op-census` builds only.)
    assert!(
        new_n > planned_n || !cfg!(any(debug_assertions, feature = "op-census")),
        "helper calls: {new_n} against {planned_n} planned"
    );
    assert_eq!(old_n, new_n, "the old name must act as the new one");
}
