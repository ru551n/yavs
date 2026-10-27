//! Element loops in compiled code (#757): on a native frame the iterations of a copy or fill loop
//! but the last run planned in the code itself (or in `yavs_rt_eloop` for the shapes it leaves), and the output, the exit status and the message of a loop
//! that leaves its vector are those of the interpreter, which runs the plain loop.
#![cfg(any(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::process::Command;

const LOOPS: &str = include_str!("loops/loops.vhd");

fn run(
    d: &std::path::Path,
    tier: &[&str],
    counters: bool,
    helper: bool,
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
    if helper {
        // The loops planned by the runtime helper instead of the code itself.
        c.env("YAVS_LLVM_ELOOP_HELPER", "1");
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
    let want = run(&d, &["--tier=interp"], false, false);
    assert!(text(&want).contains("short LLLHH000"), "{}", text(&want));
    let mut tiers = Vec::new();
    if cfg!(feature = "llvm") {
        tiers.push((vec!["--tier=llvm"], 5, false));
        tiers.push((vec!["--tier=auto", "--tier-heat=1,0,0"], 1, false));
        tiers.push((vec!["--tier=llvm"], 5, true));
        tiers.push((vec!["--tier=auto", "--tier-heat=1,0,0"], 1, true));
    }
    if cfg!(feature = "clift") {
        // The Cranelift lowering calls the helper for every element loop.
        tiers.push((vec!["--tier=clift"], 5, false));
        tiers.push((vec!["--tier=clift-auto", "--tier-heat=1,0,0"], 1, false));
    }
    for (tier, least, helper) in tiers {
        let got = run(&d, &tier, true, helper);
        assert_eq!(text(&got), text(&want), "{tier:?} helper {helper}");
        // The loops ran at once for the loops of enough elements (a build with callback counters).
        let calls = got
            .1
            .lines()
            .chain(got.2.lines())
            .find_map(|l| l.strip_prefix("counters: cb_eloop="))
            .and_then(|n| n.parse::<u64>().ok());
        if let Some(n) = calls {
            assert!(n >= least, "{tier:?} helper {helper}: {n} calls");
        }
    }
}
