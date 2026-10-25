//! Element loops in compiled code (#757): on a native frame the iterations of a copy or fill loop
//! but the last run in `yavs_rt_eloop`, and the output, the exit status and the message of a loop
//! that leaves its vector are those of the interpreter, which runs the plain loop.
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::process::Command;

const LOOPS: &str = include_str!("loops/loops.vhd");

fn run(d: &std::path::Path, tier: &[&str], counters: bool) -> (Option<i32>, String, String) {
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
    let want = run(&d, &["--tier=interp"], false);
    assert!(text(&want).contains("short LLLHH000"), "{}", text(&want));
    for (tier, least) in [
        (vec!["--tier=llvm"], 5),
        (vec!["--tier=auto", "--tier-heat=1,0,0"], 1),
    ] {
        let got = run(&d, &tier, true);
        assert_eq!(text(&got), text(&want), "{tier:?}");
        // The helper ran for the loops of enough elements (a build with callback counters).
        let calls = got
            .1
            .lines()
            .chain(got.2.lines())
            .find_map(|l| l.strip_prefix("counters: cb_eloop="))
            .and_then(|n| n.parse::<u64>().ok());
        if let Some(n) = calls {
            assert!(n >= least, "{tier:?}: {n} calls of the helper");
        }
    }
}
