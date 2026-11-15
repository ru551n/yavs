//! Cycle mode (`yavs run --cycle`, crate `yavs-cycle`): the same result lines as the normal
//! mode, with every process the mode takes in the region and the rest in the event kernel;
//! `--cycle-strict` refuses a design whose RTL below the top level cannot all join, and `--cycle`
//! refuses the options that would see inside the region.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::tempdir;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn analyze(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).expect("write design");
    let o = yavs(dir, &["analyze", "--std=2008", "--work=work:lib", file]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// The report lines of a successful run and its standard error.
fn run(dir: &Path, flags: &[&str], top: &str) -> (Vec<String>, String) {
    let mut args = vec!["run", "--work=work:lib", "--no-cache"];
    args.extend(flags);
    args.push(top);
    let o = yavs(dir, &args);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(o.status.success(), "{top} {flags:?}: {out}{err}");
    let lines = out
        .lines()
        .filter(|l| l.contains("="))
        .map(String::from)
        .collect();
    (lines, err)
}

/// The tiers of this build, each with one thread and with four.
fn tiers() -> Vec<Vec<&'static str>> {
    let mut t = vec![
        vec!["--tier=interp"],
        vec!["--tier=interp", "--threads", "4"],
    ];
    if cfg!(feature = "clift") {
        t.push(vec!["--tier=auto"]);
    }
    t
}

#[test]
fn rec_pair_gives_the_same_state_in_cycle_mode() {
    let dir = tempdir("cycle-rec-pair");
    analyze(
        &dir,
        "rec_pair.vhd",
        include_str!("../../../tools/bench/designs/rec_pair.vhd"),
    );
    // 2000 cycles: the design's own check is for its full length only, so the state is checked
    // here, hand-checked against the normal mode.
    let want = "cnt=2000 state=D566F194EC7E2DD5315B2FA5E44D5F4BA5C15738DB3BBE52006242F68BBF33E1D25306532AB808095F63C5E77C6AAA3";
    for t in tiers() {
        let (normal, _) = run(
            &dir,
            &[&t[..], &["-gCYCLES=2000"]].concat(),
            "bench_rec_pair",
        );
        for mode in ["--cycle", "--cycle-strict"] {
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["-gCYCLES=2000", mode]].concat(),
                "bench_rec_pair",
            );
            assert_eq!(cycle, normal, "{t:?} {mode}");
            assert!(normal[0].ends_with(want), "{normal:?}");
            assert!(
                err.contains("cycle mode: 2 of 3 processes run cycle-based"),
                "{err}"
            );
            assert!(err.contains("bench_rec_pair.main"), "{err}");
        }
    }
}

#[test]
fn a_region_with_processes_left_out_gives_the_same_result() {
    let dir = tempdir("cycle-partial");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_partial");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_partial");
        assert!(
            normal.len() == 1 && normal[0].ends_with("acc=907104 q=AC"),
            "{t:?} {normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        // `mix`, `early`, `late` and `reg` join; `pick` and `q <= r` stay outside, with reasons.
        assert!(
            err.contains("cycle mode: 4 of 7 processes run cycle-based"),
            "{err}"
        );
        assert!(
            err.contains("cycle_partial.dut.pick (partial.vhd:45)")
                && err.contains("an `if` statement in the body is not supported"),
            "{err}"
        );
        assert!(
            err.contains("1 wake the region (cycle_partial.din)"),
            "{err}"
        );
    }
}

/// A sensitivity list naming part of a signal is not complete for cycle mode even when it names
/// the object the body reads: the process stays out, and the result is the normal mode's.
const PART_LIST: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_part_list is
end entity;
architecture a of cycle_part_list is
  signal v : std_ulogic_vector(1 downto 0) := \"00\";
  signal w : std_ulogic_vector(1 downto 0) := \"00\";
begin
  half : process (v(0))
  begin
    w <= v;
  end process;
  main : process
  begin
    v <= \"01\";
    wait for 1 ns;
    -- Only v(1) changes: `half` does not run again in the normal mode.
    v <= \"11\";
    wait for 1 ns;
    report \"w=\" & to_string(w);
    wait;
  end process;
end architecture;
";

#[test]
fn a_sensitivity_list_naming_a_part_keeps_the_process_out() {
    let dir = tempdir("cycle-part-list");
    analyze(&dir, "part_list.vhd", PART_LIST);
    let (normal, _) = run(&dir, &["--tier=interp"], "cycle_part_list");
    let (cycle, err) = run(&dir, &["--tier=interp", "--cycle"], "cycle_part_list");
    assert!(
        normal.len() == 1 && normal[0].ends_with("w=01"),
        "{normal:?}"
    );
    assert_eq!(cycle, normal);
    assert!(
        err.contains("0 of 2 processes run cycle-based")
            && err.contains("a sensitivity list naming part of a signal"),
        "{err}"
    );
}

#[test]
fn cycle_strict_refuses_rtl_that_cannot_join() {
    let dir = tempdir("cycle-strict");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-strict",
            "cycle_partial",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(o.stdout.is_empty(), "no simulation runs");
    // Both RTL processes below the top level are listed; the testbench is exempt.
    assert!(
        err.contains("error[E4101]: process cycle_partial.dut.pick cannot run cycle-based"),
        "{err}"
    );
    assert!(err.contains("partial.vhd:45:3"), "{err}");
    assert!(
        err.contains("error[E4101]: process cycle_partial.dut.q <= ... (partial.vhd:65)"),
        "{err}"
    );
    assert!(!err.contains("cycle_partial.main"), "{err}");
}

#[test]
fn cycle_mode_refuses_the_options_that_see_inside_the_region() {
    let dir = tempdir("cycle-not-with");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for (opt, name) in [("--wave=x.fst", "--wave"), ("--cov=s", "--cov")] {
        for mode in ["--cycle", "--cycle-strict"] {
            let o = yavs(
                &dir,
                &["run", "--work=work:lib", mode, opt, "cycle_partial"],
            );
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(2), "{err}");
            assert!(
                err.contains(&format!(
                    "error[E4102]: `{mode}` cannot be used with `{name}`"
                )),
                "{err}"
            );
        }
    }
}

#[test]
fn cycle_and_cycle_strict_are_alternatives() {
    let dir = tempdir("cycle-both");
    let o = yavs(
        &dir,
        &["run", "--work=work:lib", "--cycle", "--cycle-strict", "top"],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("the argument '--cycle' cannot be used with '--cycle-strict'"),
        "{err}"
    );
}
