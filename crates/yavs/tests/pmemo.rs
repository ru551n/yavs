//! The memo of pure calls (#701, `yavs-interp` `pmemo.rs`) answers a call with arguments it has
//! seen from its table instead of running the function. The run must be the same as without it:
//! same output, same reports (a call that reports is never answered from the table), at any
//! thread count and with the processes switched to their copies early.

mod common;
use common::tempdir as tmp;

use std::path::Path;
use std::process::Command;

const CASES: &str = include_str!("pmemo/cases.vhd");

/// The output of a run of `pmemo_cases` and its counters.
fn run(dir: &Path, no_memo: bool, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    std::fs::write(dir.join("cases.vhd"), CASES).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(dir)
        .args(["analyze", "--work", &work, "cases.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(dir)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(args)
        .arg("pmemo_cases")
        .env("YAVS_COUNTERS", "1")
        .envs(env.iter().copied());
    if no_memo {
        c.env("YAVS_NO_PMEMO", "1");
    }
    let o = c.output().expect("run");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// The run's output without the work counters.
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
fn memoised_calls_change_nothing_but_the_work() {
    let d = tmp("pmemo");
    let mut cases = vec![
        (vec!["--tier=interp", "--threads", "1"], vec![]),
        (vec!["--tier=interp", "--threads", "4"], vec![]),
        (
            vec!["--tier=interp", "--threads", "1"],
            vec![("YAVS_SPEC_AFTER", "2")],
        ),
    ];
    // Compiled code asks the memo too (a probe before the call, a keep after it).
    if cfg!(feature = "llvm") {
        cases.extend([
            (vec!["--tier=llvm", "--threads", "1"], vec![]),
            (vec!["--tier=llvm", "--threads", "4"], vec![]),
            (
                vec!["--tier=llvm", "--threads", "1"],
                vec![("YAVS_SPEC_AFTER", "2")],
            ),
            (
                vec!["--tier=auto", "--tier-heat=1,0,0", "--threads", "4"],
                vec![],
            ),
        ]);
    }
    if cfg!(feature = "clift") {
        cases.extend([
            (vec!["--tier=clift", "--threads", "1"], vec![]),
            (vec!["--tier=clift", "--threads", "4"], vec![]),
        ]);
    }
    for (args, env) in cases {
        let (c0, o0, e0) = run(&d, true, &args, &env);
        let (c1, o1, e1) = run(&d, false, &args, &env);
        assert_eq!(c0, 0, "{e0}");
        assert_eq!(c1, 0, "{e1}");
        assert_eq!(output(&o0), output(&o1), "stdout, {args:?} {env:?}");
        assert_eq!(output(&e0), output(&e1), "stderr, {args:?} {env:?}");
        // The function that reports reported on every call, memoised or not.
        let (all0, all1) = (format!("{o0}{e0}"), format!("{o1}{e1}"));
        let reports = |e: &str| e.matches("chk: b is three").count();
        assert_eq!(reports(&all0), reports(&all1));
        assert!(reports(&all1) >= 30, "{all1}");
        assert_eq!(counter(&e0, "pmemo_hits"), 0);
        assert!(counter(&e1, "pmemo_hits") > 100, "{e1}");
        assert!(all1.contains("acc="), "{all1}");
    }
}
