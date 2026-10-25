//! Element loops (#520): loops over the elements of logic vectors that copy or fill run all
//! iterations but the last at once when every index of them is in range, and the generic ops run
//! the rest, so results and failures are those of the plain loop. Each function below is a
//! loop of a different shape; the test checks the results, that the iterations ran at once
//! (`YAVS_DUMPIR` says what each entry of a loop did) and the message of a loop that leaves the vector.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const LOOPS: &str = include_str!("loops/loops.vhd");

fn run(d: &std::path::Path, env_dump: bool) -> (String, String) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(d)
        .args(["analyze", "--work", &work, "loops.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(d)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--tier=interp", "loops"]);
    if env_dump {
        c.env("YAVS_DUMPIR", "1");
    }
    let o = c.output().expect("run");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn element_loops_give_the_results_of_the_plain_loops() {
    let d = tmp("loops");
    std::fs::write(d.join("loops.vhd"), LOOPS).expect("write");
    let (out, err) = run(&d, true);
    let all = format!("{out}\n{err}");
    // Worked out by hand from the sources above.
    for want in [
        "copy 10XZ01LH-WU1100X",
        "copy70 1100110011001100110011001100110011001100110011001100110011001100110011",
        "rev X0011UW-HL10ZX01",
        "fill 1111111111111111111111111111111111111111",
        "evens ---W-U-1-1-0-0-X",
        "row1 H-WU1100",
        "row2 LH-WU110",
        "over 10XZ01LH-WU1100X",
        "shift 1111111111111111",
        "shiftdown 0XZ01LH-WU1100XX",
        "short LLLHH000",
    ] {
        assert!(all.contains(want), "no `{want}`\n{all}");
    }
    // The loops over more than two elements ran at once, all iterations but the last.
    let at_once: Vec<_> = err
        .lines()
        .filter(|l| l.contains("iterations at once"))
        .collect();
    for want in [
        "15 of 15", "69 of 69", "39 of 39", "7 of 7", "14 of 14", "16 of 16",
    ] {
        assert!(
            at_once.iter().any(|l| l.contains(want)),
            "no loop with {want} iterations at once\n{err}"
        );
    }
    // A loop that leaves the vector fails at the same index as the plain loop would.
    assert!(
        all.contains("index 16 outside of range 15 downto 0 for a"),
        "{all}"
    );
}
