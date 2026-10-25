//! Result tables for pure functions of one small discrete argument (#722, `yavs-ir`
//! `casetab.rs`): a run with the tables must print what a run without them prints, reports
//! included, at any thread count and in every tier.

mod common;
use common::tempdir as tmp;

use std::path::Path;
use std::process::Command;

const CASES: &str = include_str!("casetab/cases.vhd");

/// Exit status, stdout and stderr of a run of `casetab_cases`.
fn run(dir: &Path, off: bool, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    std::fs::write(dir.join("cases.vhd"), CASES).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(dir)
        .args(["analyze", "--std=2008", "--work", &work, "cases.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(dir)
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(args)
        .arg("casetab_cases")
        .env("YAVS_COUNTERS", "1")
        .envs(env.iter().copied());
    if off {
        c.env("YAVS_NO_CASETAB", "1");
    }
    let o = c.output().expect("run");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn output(s: &str) -> String {
    s.lines()
        .filter(|l| !l.starts_with("counters:"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn counter(s: &str, name: &str) -> u64 {
    let key = format!("counters: {name}=");
    s.lines()
        .find_map(|l| l.strip_prefix(&key))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name} in:\n{s}"))
}

#[test]
fn tables_change_nothing_but_the_work() {
    let d = tmp("casetab");
    let mut cases = vec![
        (vec!["--tier=interp", "--threads", "1"], vec![]),
        (vec!["--tier=interp", "--threads", "4"], vec![]),
    ];
    if cfg!(feature = "llvm") {
        cases.extend([
            (vec!["--tier=llvm", "--threads", "1"], vec![]),
            (
                vec!["--tier=auto", "--tier-heat=1,0,0", "--threads", "4"],
                vec![],
            ),
        ]);
    }
    for (args, env) in cases {
        let (c0, o0, e0) = run(&d, true, &args, &env);
        let (c1, o1, e1) = run(&d, false, &args, &env);
        assert_eq!(c0, 0, "{e0}");
        assert_eq!(c1, 0, "{e1}");
        assert_eq!(output(&o0), output(&o1), "stdout, {args:?} {env:?}");
        assert_eq!(output(&e0), output(&e1), "stderr, {args:?} {env:?}");
        let all = format!("{o1}{e1}");
        assert!(all.contains("casetab a="), "{all}");
        // 3 rounds of the 234 characters that are no hex digit, 3 of 10 operations: every
        // report is still made.
        assert_eq!(
            all.matches("hexcheck: bad character").count(),
            3 * 234,
            "{all}"
        );
        assert_eq!(all.matches("opwarn: sub").count(), 3, "{all}");
        assert_eq!(all.matches("opwarn: shr").count(), 3, "{all}");
        // The tables answer the calls: fewer reach the memo of pure calls.
        let calls = |e: &str| counter(e, "pmemo_hits") + counter(e, "pmemo_runs");
        assert!(calls(&e1) < calls(&e0), "{e0}\n{e1}");
    }
}
