//! Cycle mode (`yavs run --cycle`, crate `yavs-cycle`): the same result lines as the normal
//! mode, with every process the mode takes in the region and the rest in the event kernel;
//! `--cycle-strict` refuses a design whose RTL below the top level cannot all join, and `--cycle`
//! refuses the options that would see inside the region.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::tempdir;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    yavs_env(dir, args, &[])
}

/// `yavs` with more environment (an empty value removes a variable).
fn yavs_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        // Every import read again and compared: an input the region missed is an error.
        .env("YAVS_CYCLE_IMPORT_CHECK", "1")
        // Every combinational step behind a guard, every step skipped run anyway and compared,
        // and every run compared with one of every step: a skip on inputs that did change is an
        // error.
        .env("YAVS_CYCLE_GUARD", "all")
        .env("YAVS_CYCLE_SKIP_CHECK", "full");
    for &(k, v) in env {
        if v.is_empty() {
            c.env_remove(k);
        } else {
            c.env(k, v);
        }
    }
    c.args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn analyze(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).expect("write design");
    let o = yavs(dir, &["analyze", "--std=2008", "--work=work:lib", file]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// `err`, the standard error of a run with `flags`, followed by the region report of the same
/// design when it ran in cycle mode (a `--cycle-report` run, which analyses and stops): the
/// membership rows are read there, a cycle run prints one line.
fn with_report(dir: &Path, flags: &[&str], top: &str, mut err: String) -> String {
    if flags
        .iter()
        .any(|f| matches!(*f, "--cycle" | "--cycle-strict"))
    {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags.iter().filter(|f| !f.starts_with("--cycle")));
        args.extend(["--cycle-report", top]);
        let o = yavs(dir, &args);
        let report = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success() && o.stdout.is_empty(), "{top}: {report}");
        err.push_str(&report);
    }
    err
}

/// The report lines of a successful run and its standard error.
fn run(dir: &Path, flags: &[&str], top: &str) -> (Vec<String>, String) {
    run_vars(dir, flags, top, &[])
}

fn run_vars(dir: &Path, flags: &[&str], top: &str, env: &[(&str, &str)]) -> (Vec<String>, String) {
    let mut args = vec!["run", "--work=work:lib", "--no-cache"];
    args.extend(flags);
    args.push(top);
    let o = yavs_env(dir, &args, env);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(o.status.success(), "{top} {flags:?}: {out}{err}");
    let err = with_report(dir, flags, top, err);
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
        // `mix`, `early`, `late`, `reg` and `q <= r` join; `pick` stays outside, with its reason.
        assert!(
            err.contains("cycle mode: 5 of 7 processes run cycle-based"),
            "{err}"
        );
        assert!(
            err.contains("cycle_partial.dut.pick (partial.vhd:66)")
                && err.contains("a `while` loop or a loop without a scheme is not supported"),
            "{err}"
        );
        assert!(
            err.contains("inputs, 1 wake the region (cycle_partial.din)"),
            "{err}"
        );
    }
}

#[test]
fn region_outputs_read_late_waited_on_or_read_by_an_idle_skipped_process() {
    let dir = tempdir("cycle-quiet");
    analyze(&dir, "quiet.vhd", include_str!("cycle/quiet.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_quiet");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_quiet");
        // Hand-checked: `nxt` changes at time 0 and at each of the 20 edges, `events` starts at 0
        // and counts one more for the initial run of `watch`.
        assert!(
            normal.len() == 1 && normal[0].ends_with("cnt=20 last=19 seen=19 events=22"),
            "{t:?} {normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 5 processes run cycle-based"),
            "{err}"
        );
        // `cnt` and `last` are read by value only, `nxt` is waited on.
        assert!(
            err.contains("3 signal leaves exported (2 nothing outside waits on)"),
            "{err}"
        );
    }
}

#[test]
fn region_outputs_seen_every_clock_on_events_and_through_local_subprograms() {
    let dir = tempdir("cycle-export");
    analyze(&dir, "export.vhd", include_str!("cycle/export.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_export");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_export");
        // Hand-checked: 40 edges, `v` is `cnt` xor `cnt` rotated by one, `w` a Johnson counter
        // (40 ones), `pulse` has no event after the initial run of `on_pulse`; `hw` changes in
        // its high word only (an event at time 0 and at every edge), the driver of `mid(79 downto
        // 20)` only above element 63 (at every edge), `pk.b(9 downto 6)` after 16 and 32 edges;
        // `sum` holds them: 276, 276 and 32.
        assert!(
            normal.len() == 2
                && normal[0].ends_with(
                    "cnt=28 v=3C ra=0 rb=A w=000000000000000FFFFFFFFFF both=2B acc=58 sum=6062 \
                     ones=20020 nd=16 nn=8"
                )
                && normal[1]
                    .ends_with("events v3=5 ra=32 w=41 pulse=1 p=3 f=2 cnt=2 hw=42 pk=3 mid=41"),
            "{t:?} {normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 11 processes run cycle-based"),
            "{err}"
        );
        // Exported: every output but `inner`, which only members read; the testbench calls local
        // subprograms, whose bodies name `viaf` (read), `viad` in an initial value, `vian` in an
        // allocator's, `viap` and through a formal `viaformal` (waited on), and the predefined `=`
        // of `state_t`. Nothing outside waits on `both`, `acc`, `viaf`, `viad`, `vian` and `st`.
        assert!(
            err.contains("17 signal leaves exported (6 nothing outside waits on)"),
            "{err}"
        );
    }
}

#[test]
fn external_names_export_only_what_they_name() {
    // One library each: a package in the library is part of every run's design.
    let dir = tempdir("cycle-xname");
    analyze(&dir, "xname.vhd", include_str!("cycle/xname.vhd"));
    analyze(&dir, "xname2.vhd", include_str!("cycle/xname2.vhd"));
    let pdir = tempdir("cycle-xnamepkg");
    analyze(&pdir, "xnamepkg.vhd", include_str!("cycle/xnamepkg.vhd"));
    // Hand-checked: `cnt` steps by 3, `hid` takes `hid xor cnt` at each of 20 edges from 5A;
    // the sums are of `dbl + 3 * cnt` and of `hid` after each edge; `fast` changes at time 0
    // and at every edge. Exported: `q`, `dbl` and `cnt`; `fast` stays out with the actual that
    // reads it (an external name in a member is not followed), and its readers' result may depend
    // on its deltas. Through the package procedure every signal of the design is exported.
    let cases = [
        (
            &dir,
            "cycle_xname",
            "sum=4304 q=66 fast=21 last=3D",
            "3 of 7",
            "3 signal leaves exported (2 nothing",
        ),
        // `fast` waited on directly, `dbl` by the concurrent assertion.
        (
            &dir,
            "cycle_xname_wait",
            "fast=21",
            "4 of 7",
            "2 signal leaves exported (0 nothing",
        ),
        (
            &dir,
            "cycle_xname_fn",
            "sum=1784",
            "4 of 5",
            "5 signal leaves exported (0 nothing",
        ),
        (
            &dir,
            "cycle_xname_lpkg",
            "sum=1784",
            "4 of 5",
            "5 signal leaves exported (0 nothing",
        ),
        (
            &pdir,
            "cycle_xname_pkg",
            "sum=1784",
            "4 of 5",
            "5 signal leaves exported (0 nothing",
        ),
    ];
    for (d, top, sum, members, exported) in cases {
        for t in tiers() {
            let (normal, _) = run(d, &t, top);
            assert!(
                normal.last().is_some_and(|l| l.ends_with(sum)),
                "{t:?} {normal:?}"
            );
            let (cycle, err) = run(d, &[&t[..], &["--cycle"]].concat(), top);
            assert_eq!(cycle, normal, "{t:?}\n{err}");
            assert!(
                err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                "{err}"
            );
            assert!(err.contains(exported), "{err}");
        }
    }
}

/// A subprogram of a package declared in an architecture reads that architecture's signals: the
/// testbench calling it reads `acc`, which must be exported (it read 0 in cycle mode before).
#[test]
fn a_package_declared_in_an_architecture_reads_its_signals() {
    let dir = tempdir("cycle-localpkg");
    analyze(&dir, "localpkg.vhd", include_str!("cycle/localpkg.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_localpkg");
        // Hand-checked: `acc` is 5 times the edge count, 5 * (1 + ... + 20).
        assert!(
            normal.len() == 1 && normal[0].ends_with("sum=1050"),
            "{t:?} {normal:?}"
        );
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_localpkg");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 3 processes run cycle-based")
                && err.contains("1 signal leaves exported (1 nothing outside waits on)"),
            "{err}"
        );
    }
}

/// Owner decision A29 as it stands: an 'X' from a process outside, copied by a member to a signal
/// the testbench reads, reads as '0' there. A change of this rule changes this test.
#[test]
fn an_x_from_outside_copied_by_a_member_reads_as_0() {
    let dir = tempdir("cycle-xcopy");
    analyze(&dir, "xcopy.vhd", include_str!("cycle/xcopy.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_xcopy");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_xcopy");
        let ends = |v: &[String]| {
            v.iter()
                .map(|l| l.rsplit(' ').next().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ends(&normal), ["pin='X'", "pin='0'"], "{t:?} {normal:?}");
        assert_eq!(
            ends(&cycle),
            ["pin='0'", "pin='0'"],
            "{t:?} {cycle:?}\n{err}"
        );
        assert!(
            err.contains("cycle mode: 1 of 4 processes run cycle-based")
                && err.contains("cycle_xcopy.copy (xcopy.vhd:21)"),
            "{err}"
        );
    }
}

#[test]
fn inputs_settled_at_the_next_edge_give_the_same_result() {
    let dir = tempdir("cycle-lazy");
    analyze(&dir, "lazy.vhd", include_str!("cycle/lazy.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_lazy");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_lazy");
        // Hand-checked: 24 edges; the bus model sees `ready` in 6 of its 10 handshakes (`busy`
        // is `cnt(1)` of the edge before); the monitor's `q` follows `a xor ae xor b` an edge
        // late (1 at 11 ns: `b` was 0, `a` toggled after the first edge and back twice by 3 ns);
        // `r` is reset to `rv` while `ext` and not `q2` hold, from 4 ns after an edge: 0 at 5 ns,
        // 1 at 6 ns after `rv` toggled, 1 at 15 ns (`rv` still 1).
        let has = |s: &str| normal.iter().any(|l| l.ends_with(s));
        assert!(
            normal.len() == 41
                && has("11000000fs+0: q=1 q2=0 tq=2 ready=0")
                && has("5000000fs+0: r=0")
                && has("6000000fs+0: r=1")
                && has("15000000fs+0: r=1")
                && normal[40].ends_with("q=0 q2=0 tq=4 handshakes=6"),
            "{t:?} {normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 6 of 11 processes run cycle-based"),
            "{err}"
        );
        // `valid` reaches an output the bus model reads, `sel` the index check of `look`, `ext`
        // the register `r` through the reset it makes, `rv` through the reset branch; `a`, `ae`
        // and `b` reach only the register `q`.
        assert!(
            err.contains(
                "wakes: 4 settled at once, through combinational code to what outside sees \
                 (cycle_lazy.valid to cycle_lazy.ready read by cycle_lazy.bus_model, \
                 cycle_lazy.sel to a check in cycle_lazy.look, cycle_lazy.ext to cycle_lazy.r \
                 read by cycle_lazy.rmon, cycle_lazy.rv to cycle_lazy.r read by cycle_lazy.rmon); \
                 3 read when they change and settled at the next edge"
            ),
            "{err}"
        );
    }
    // The experiment that publishes outputs at clock edges only: the bus model never sees
    // `ready` in the time step of its `valid`.
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&dir)
        .env("YAVS_STD_CACHE", "off")
        .env("YAVS_CYCLE_EDGE_OUTPUTS", "1")
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle",
        ])
        .arg("cycle_lazy")
        .output()
        .expect("run yavs");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("q=0 q2=0 tq=4 handshakes=0"), "{out}");
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
fn an_integer_and_index_heavy_process_gives_the_same_result() {
    let dir = tempdir("cycle-ints");
    analyze(&dir, "ints.vhd", include_str!("cycle/ints.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_ints");
        // Hand-checked against the normal mode.
        assert!(
            normal.len() == 1
                && normal[0]
                    .ends_with("cnt=2000 sum=57616 acc=FF5E6AEF ring=1CBC01045F1F0FCAEEE0C3561"),
            "{t:?} {normal:?}"
        );
        for mode in ["--cycle", "--cycle-strict"] {
            let (cycle, err) = run(&dir, &[&t[..], &[mode]].concat(), "cycle_ints");
            assert_eq!(cycle, normal, "{t:?} {mode}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{err}"
            );
        }
    }
}

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
    // Element by element: `half` reads `v(1)`, which its list does not name.
    assert!(
        err.contains("0 of 2 processes run cycle-based")
            && err.contains(
                "E4121 it reads cycle_part_list.v(1), which its sensitivity list does not name"
            ),
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
    // The RTL process below the top level that cannot join is listed; the testbench is exempt.
    assert!(
        err.contains("error[E4101]: process cycle_partial.dut.pick cannot run cycle-based yet"),
        "{err}"
    );
    assert!(err.contains("partial.vhd:66:3"), "{err}");
    // The concurrent `q <= r` joins.
    assert!(!err.contains("cycle_partial.dut.q <= ..."), "{err}");
    assert!(!err.contains("cycle_partial.main"), "{err}");
}

#[test]
fn cycle_mode_refuses_the_options_that_see_inside_the_region() {
    let dir = tempdir("cycle-not-with");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for (opt, name) in [
        ("--wave=x.fst", "--wave"),
        ("--cov=s", "--cov"),
        ("--vhpi=nothing", "--vhpi"),
        ("--surfer", "--surfer"),
        ("--surfer=localhost:1", "--surfer"),
        ("--wave=x.fst --surfer", "--surfer"),
        ("--activity", "--activity"),
        ("--activity=x.json", "--activity"),
    ] {
        for mode in ["--cycle", "--cycle-strict"] {
            let mut args = vec!["run", "--work=work:lib", mode];
            args.extend(opt.split(' '));
            args.push("cycle_partial");
            let o = yavs(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(2), "{opt}: {err}");
            assert!(
                err.contains(&format!(
                    "error[E4102]: `{mode}` cannot be used with `{name}`"
                )) && err.contains(&format!("run without `{mode}`")),
                "{opt}: {err}"
            );
            // Refused before any work: no wave file, no simulation.
            assert!(!dir.join("cycle_partial.fst").exists() && !dir.join("x.fst").exists());
            assert!(o.stdout.is_empty(), "{opt}");
        }
    }
}

/// What stays allowed with `--cycle`: options that do not read the signals of the region.
#[test]
fn cycle_mode_allows_the_options_that_do_not_see_inside_the_region() {
    let dir = tempdir("cycle-allowed");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for opt in [
        "--profile",
        "--assert-summary",
        "--shuffle",
        "--hang-report",
        "--time-report",
    ] {
        let o = yavs(
            &dir,
            &["run", "--work=work:lib", "--cycle", opt, "cycle_partial"],
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{opt}: {err}");
        assert!(!err.contains("E4102"), "{opt}: {err}");
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

#[test]
fn cycle_report_is_a_third_mode_exclusive_with_the_other_two() {
    let dir = tempdir("cycle-report-excl");
    for other in ["--cycle", "--cycle-strict"] {
        let o = yavs(
            &dir,
            &["run", "--work=work:lib", other, "--cycle-report", "top"],
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{other}: {err}");
        assert!(
            err.contains("cannot be used with") && err.contains("--cycle-report"),
            "{other}: {err}"
        );
    }
}

#[test]
fn cycle_report_prints_the_report_and_runs_nothing() {
    let dir = tempdir("cycle-report");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    // The testbench would print `acc=... q=...`; nothing runs, so nothing is printed.
    let report = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
        args.extend(flags);
        args.push("cycle_partial");
        yavs(&dir, &args)
    };
    let o = report(&["--cycle-report"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success() && o.stdout.is_empty(), "{err}");
    assert!(
        err.starts_with("cycle mode: 5 of 7 processes run cycle-based")
            && err.contains("cycle_partial.dut.pick (partial.vhd:66)"),
        "{err}"
    );
    // With a file the report goes there.
    let file = dir.join("report.txt");
    let o = report(&[&format!("--cycle-report={}", file.display())]);
    assert!(o.status.success() && o.stdout.is_empty() && o.stderr.is_empty());
    let text = std::fs::read_to_string(&file).expect("the report file");
    assert_eq!(text, err);
    // An unwritable file fails before any work: not even the top-level name is looked up.
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle-report=no/such/dir/r.txt",
            "nothing",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("error[E4103]: cannot write the cycle report file `no/such/dir/r.txt`"),
        "{err}"
    );
}

#[test]
fn plain_cycle_prints_one_summary_line() {
    let dir = tempdir("cycle-summary");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle",
            "cycle_partial",
        ],
    );
    // One line, then the count of metavalues the region read as 0, which is its own line.
    let err = String::from_utf8_lossy(&o.stderr);
    let mut lines = err.lines();
    assert_eq!(
        lines.next(),
        Some(
            "cycle mode: 5 of 7 processes run cycle-based, 2 in the event kernel (see --cycle-report)"
        )
    );
    assert!(
        lines.all(|l| l.contains("metavalue elements read as 0")),
        "{err}"
    );
}

#[test]
fn a_skip_check_value_it_does_not_take_is_an_error() {
    let dir = tempdir("cycle-skipval");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for (v, ok) in [("1", true), ("full", true), ("ful", false), ("yes", false)] {
        let args = [
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle",
            "cycle_partial",
        ];
        let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_SKIP_CHECK", v)]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.success(), ok, "{v}: {err}");
        if !ok {
            let want =
                format!("error[E4104]: YAVS_CYCLE_SKIP_CHECK takes `1` or `full`, not `{v}`");
            assert!(err.contains(&want), "{err}");
            assert_eq!(
                err.contains("similar name exists: `full`"),
                v == "ful",
                "{err}"
            );
        }
    }
}

#[test]
fn the_report_groups_what_stays_out_by_reason_most_common_first() {
    let dir = tempdir("cycle-groups");
    analyze(&dir, "subset.vhd", include_str!("cycle/subset.vhd"));
    let report = verdicts(&dir, "sub_tb");
    let counts: Vec<(usize, &str)> = (report.lines())
        .filter_map(|l| {
            let l = l.strip_prefix("    ")?.strip_suffix("):")?;
            let (head, n) = l.rsplit_once(" (")?;
            Some((n.split(' ').next()?.parse().ok()?, head))
        })
        .collect();
    // Every code, the behavioural group and the one not implemented yet.
    assert!(counts.len() > 15, "{report}");
    assert!(counts.windows(2).all(|w| w[0].0 >= w[1].0), "{counts:?}");
    // First the logic a process with a foreign call may read (`calls_foreign`: every signal).
    assert!(counts[0].1.starts_with("E4101 "), "{counts:?}");
    assert!(
        report.contains("    behavioural, in the event kernel (6 processes):"),
        "{report}"
    );
    assert!(
        report.contains("cycle-based processes give no warnings of the ieee libraries"),
        "{report}"
    );
    // The processes of a group are listed under its heading.
    let i = report.find("    E4118 ").expect("a group of E4118");
    assert!(
        report[i..].contains("\n      sub_tb.dut.high_z "),
        "{report}"
    );
}

/// The two-state operations of the region against the normal mode on 0/1 inputs, at widths
/// around the word size. Each width is its own design (the lowering takes folded bounds only).
fn ops_design(w: u32) -> String {
    let rot = if w >= 2 {
        "      y_rot <= a(0) & a(W - 1 downto 1);\n"
    } else {
        "      y_rot <= a;\n"
    };
    "\
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_ops is
end entity;
architecture a of cycle_ops is
  signal clk : std_ulogic := '0';
  signal a, b : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal i : integer range 0 to W - 1 := 0;
  signal y_and, y_or, y_xor, y_nand, y_nor, y_xnor, y_not : std_ulogic_vector(W - 1 downto 0);
  signal y_add, y_sub, y_inc, y_ins, y_rot : std_ulogic_vector(W - 1 downto 0);
  signal y_cat : std_ulogic_vector(2 * W - 1 downto 0);
  signal y_eq, y_ne : boolean;
  signal y_bit : std_ulogic;
  signal r : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  ops : process (a, b, i)
  begin
      y_and <= a and b;
      y_or <= a or b;
      y_xor <= a xor b;
      y_nand <= a nand b;
      y_nor <= a nor b;
      y_xnor <= a xnor b;
      y_not <= not a;
      y_add <= std_ulogic_vector(unsigned(a) + unsigned(b));
      y_sub <= std_ulogic_vector(unsigned(a) - unsigned(b));
      y_inc <= std_ulogic_vector(unsigned(a) + 1);
      y_ins <= b;
      y_ins(W - 1) <= a(0);
ROT      y_cat <= a & b;
      y_eq <= a = b;
      y_ne <= a /= b;
      y_bit <= a(i);
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r(i) <= a(i) xor b(W - 1 - i);
      i <= (i + 1) mod W;
    end if;
  end process;
  main : process
    variable x : unsigned(63 downto 0) := x\"9E3779B97F4A7C15\";
    variable va, vb : std_ulogic_vector(W - 1 downto 0);
  begin
    for c in 1 to 12 loop
      for k in 0 to W - 1 loop
        va(k) := x(k mod 64) xor x((k * 7 + 3) mod 64);
        vb(k) := x((k * 5 + 1) mod 64);
      end loop;
      -- Equal operands now and then, for `=`.
      if c mod 4 = 0 then
        vb := va;
      end if;
      a <= va;
      b <= vb;
      x := (x(62 downto 0) & '0') xor (x(63 downto 0) and x\"000000000000001B\" and (63 downto 0 => x(63)));
      x := x + 16#5851F42D#;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report \"c=\" & integer'image(c) & \" and=\" & to_hstring(y_and) & \" or=\" & to_hstring(y_or)
        & \" xor=\" & to_hstring(y_xor) & \" nand=\" & to_hstring(y_nand) & \" nor=\" & to_hstring(y_nor)
        & \" xnor=\" & to_hstring(y_xnor) & \" not=\" & to_hstring(y_not) & \" add=\" & to_hstring(y_add)
        & \" sub=\" & to_hstring(y_sub) & \" inc=\" & to_hstring(y_inc) & \" ins=\" & to_hstring(y_ins)
        & \" rot=\" & to_hstring(y_rot) & \" cat=\" & to_hstring(y_cat) & \" eq=\" & boolean'image(y_eq)
        & \" ne=\" & boolean'image(y_ne) & \" bit=\" & to_string(y_bit) & \" r=\" & to_hstring(r);
    end loop;
    wait;
  end process;
end architecture;
"
    .replace("ROT", rot)
    .replace("2 * W - 1", &format!("{}", 2 * w - 1))
    .replace("W - 1", &format!("{}", w - 1))
    .replace("mod W", &format!("mod {w}"))
}

#[test]
fn two_state_operations_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-ops");
    for w in [1, 63, 64, 65, 128, 200] {
        analyze(&dir, &format!("ops{w}.vhd"), &ops_design(w));
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_ops");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_ops");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// All nine values at a region input, a metavalue literal and registers without an initial
/// value: two-state reads `'1'` and `'H'` as 1 and everything else as 0, the outputs go back as
/// `'0'` and `'1'`, and the metavalues read as 0 are counted (hand-checked).
const CODES: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_codes is
end entity;
architecture a of cycle_codes is
  signal clk : std_ulogic := '0';
  signal din : std_ulogic_vector(8 downto 0) := \"UX01ZWLH-\";
  signal q : std_ulogic_vector(8 downto 0);
  signal lit, lit_i : std_ulogic_vector(3 downto 0);
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= din;
    end if;
  end process;
  -- The literal reaches the testbench through a member that computes with it: two-state.
  regl : process (clk)
  begin
    if rising_edge(clk) then
      lit_i <= \"X1U0\";
    end if;
  end process;
  lit <= lit_i or \"0000\";
  main : process
  begin
    for i in 1 to 2 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report \"q=\" & to_string(q) & \" lit=\" & to_string(lit);
    end loop;
    wait;
  end process;
end architecture;
";

#[test]
fn boundary_values_and_metavalues_become_two_state_and_are_counted() {
    let dir = tempdir("cycle-codes");
    analyze(&dir, "codes.vhd", CODES);
    let (normal, _) = run(&dir, &["--tier=interp"], "cycle_codes");
    assert!(normal[0].ends_with("q=UX01ZWLH- lit=X1U0"), "{normal:?}");
    for t in tiers() {
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_codes");
        // U X 0 1 Z W L H - in, 0 0 0 1 0 0 0 1 0 out: the change of two-state values on purpose.
        assert_eq!(cycle.len(), 2, "{cycle:?}");
        assert!(
            cycle.iter().all(|l| l.ends_with("q=000100010 lit=0100")),
            "{t:?} {cycle:?}"
        );
        assert!(
            err.contains("clocked, 1 literal with a metavalue read as 0"),
            "{err}"
        );
        // At initialisation: 5 in `din`, 9 in `q`, 4 in `lit` and 4 in `lit_i` (no initial
        // value); `din` never changes, so it is not read again.
        assert!(
            err.contains(
                "cycle mode: 22 metavalue elements read as 0 (22 at initialisation, 0 at changes \
                 of the region's inputs)"
            ),
            "{err}"
        );
    }
}

/// The overflow, range and index checks of region code stop the run as the normal mode does.
const CHECKS: &str = "\
entity cycle_ovf is
end entity;
architecture a of cycle_ovf is
  signal clk : bit := '0';
  signal x : integer := 1;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x * 3;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_rng is
end entity;
architecture a of cycle_rng is
  signal clk : bit := '0';
  signal x : integer range 0 to 7 := 0;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x + 1;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_val is
end entity;
architecture a of cycle_val is
  type e_t is (aa, bb, cc, dd, ee);
  signal clk : bit := '0';
  signal x : integer range 0 to 9 := 0;
  signal y : e_t := aa;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= (x + 1) mod 10;
      y <= e_t'val(x);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_tblidx is
end entity;
architecture a of cycle_tblidx is
  type tbl_t is array (0 to 7) of natural;
  constant TBL : tbl_t := (3, 1, 4, 1, 5, 9, 2, 6);
  signal clk : bit := '0';
  signal i : integer range 0 to 15 := 0;
  signal z : natural := 0;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      i <= i + 1;
      z <= TBL(i);
    end if;
  end process;
  main : process
  begin
    for k in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_idx is
end entity;
architecture a of cycle_idx is
  signal clk : bit := '0';
  signal x : integer := 0;
  signal v : bit_vector(7 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x + 1;
      v(x) <= '1';
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_ramidx is
end entity;
architecture a of cycle_ramidx is
  signal clk : bit := '0';
  signal x : integer := 4192;
  signal v : bit_vector(4199 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x + 1;
      v(x) <= '1';
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;
";

#[test]
fn failed_checks_in_region_code_stop_the_run_as_in_the_normal_mode() {
    let dir = tempdir("cycle-checks");
    analyze(&dir, "checks.vhd", CHECKS);
    for (top, msg, note) in [
        (
            "cycle_ovf",
            "error[E3009]: result of 1162261467 * 3 cannot be represented as INTEGER",
            "at 190 ns, delta 1, in process cycle_ovf.reg",
        ),
        (
            "cycle_rng",
            "error[E3009]: value 8 outside of INTEGER range 0 to 7 for signal X",
            "at 70 ns, delta 1, in process cycle_rng.reg",
        ),
        (
            "cycle_idx",
            "error[E3009]: index 8 outside of range 7 downto 0 for v",
            "at 80 ns, delta 1, in process cycle_idx.reg",
        ),
        (
            "cycle_tblidx",
            "error[E3009]: index 8 outside of range 0 to 7 for TBL",
            "at 80 ns, delta 1, in process cycle_tblidx.reg",
        ),
        (
            "cycle_val",
            "error[E3009]: value 5 outside of E_T range 0 to 4",
            "at 50 ns, delta 1, in process cycle_val.reg",
        ),
        // A memory: the write waits for the commit, its index check fails there.
        (
            "cycle_ramidx",
            "error[E3009]: index 4200 outside of range 4199 downto 0 for v",
            "at 80 ns, delta 1, in process cycle_ramidx.reg",
        ),
    ] {
        let mut shown = Vec::new();
        for mode in [None, Some("--cycle")] {
            let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
            args.extend(mode);
            args.push(top);
            let o = yavs(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "{top} {mode:?}: {err}");
            assert!(
                err.contains(msg) && err.contains(note),
                "{top} {mode:?}: {err}"
            );
            shown.push(err[err.find("error[").expect("an error")..].to_string());
            if mode.is_some() {
                assert!(
                    err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                    "{err}"
                );
            }
        }
        // The same diagnostic, with the place of the statement and of the calls around it.
        assert_eq!(shown[0], shown[1], "{top}");
    }
}

const CALL_CHECKS: &str = "\
entity cycle_fretc is
end entity;
architecture a of cycle_fretc is
  signal clk : bit := '0';
  signal x : integer := 5;
  function dec(v : integer) return natural is
  begin
    return v - 1;
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= dec(0);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_fparc is
end entity;
architecture a of cycle_fparc is
  signal clk : bit := '0';
  signal y : integer := 0;
  function inc(v : natural) return integer is
  begin
    return v + 1;
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      y <= inc(-1);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_eslice is
end entity;
architecture a of cycle_eslice is
  type arr_t is array (0 to 3) of bit_vector(7 downto 0);
  signal clk : bit := '0';
  signal k : integer := 2;
  signal o : bit_vector(7 downto 0);
begin
  reg : process (clk)
    variable v : arr_t := (others => x\"00\");
  begin
    if clk'event and clk = '1' then
      k <= k - 1;
      v(k)(3 downto 0) := \"1010\";
      o <= v(0);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 8 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_aelem is
end entity;
architecture a of cycle_aelem is
  type nat_arr is array (0 to 3) of natural range 0 to 9;
  signal clk : bit := '0';
  signal x : natural := 0;
  signal o : natural := 0;
begin
  reg : process (clk)
    variable v : nat_arr := (others => 0);
  begin
    if clk'event and clk = '1' then
      x <= x + 1;
      v(x mod 4) := v(x mod 4) + 1;
      o <= v(1);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_fret is
end entity;
architecture a of cycle_fret is
  signal clk : bit := '0';
  signal x : integer := 5;
  function dec(v : integer) return natural is
  begin
    return v - 1;
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= dec(x);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_fpar is
end entity;
architecture a of cycle_fpar is
  signal clk : bit := '0';
  signal x, y : integer := 0;
  function inc(v : natural) return integer is
  begin
    return v + 1;
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x - 1;
      y <= inc(x + 3);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_fback is
end entity;
architecture a of cycle_fback is
  signal clk : bit := '0';
  signal x : integer range 0 to 7 := 0;
  function nine(v : integer) return integer is
  begin
    return v + 9;
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= nine(x);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 4 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

entity cycle_fidx is
end entity;
architecture a of cycle_fidx is
  signal clk : bit := '0';
  signal x : integer := 0;
  signal v : bit_vector(7 downto 0) := x\"5A\";
  signal o : bit;
  function bit_at(d : bit_vector; i : integer) return bit is
  begin
    return d(i);
  end function;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x + 1;
      o <= bit_at(v, x);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;

";

/// The checks inside an inlined call (a result outside its subtype, an actual outside its
/// formal's, an index outside a formal's range), and of an element of an array of integers, stop
/// the run with the normal mode's message.
#[test]
fn failed_checks_in_inlined_calls_stop_the_run_as_in_the_normal_mode() {
    let dir = tempdir("cycle-call-checks");
    analyze(&dir, "checks.vhd", CALL_CHECKS);
    for (top, msg, note) in [
        (
            "cycle_fret",
            "error[E3009]: value -1 outside of NATURAL range 0 to 2147483647 for the result of dec",
            "at 50 ns, delta 1, in process cycle_fret.reg",
        ),
        (
            "cycle_fpar",
            "error[E3009]: value -1 outside of NATURAL range 0 to 2147483647 for parameter V",
            "at 40 ns, delta 1, in process cycle_fpar.reg",
        ),
        (
            "cycle_fidx",
            "error[E3009]: index 8 outside of range 7 downto 0 for d",
            "at 80 ns, delta 1, in process cycle_fidx.reg",
        ),
        (
            "cycle_aelem",
            "error[E3009]: value 10 outside of INTEGER range 0 to 9 for variable V",
            "at 360 ns, delta 1, in process cycle_aelem.reg",
        ),
        // A call on constants is folded only when no check of it fails.
        (
            "cycle_fretc",
            "error[E3009]: value -1 outside of NATURAL range 0 to 2147483647 for the result of dec",
            "at 0 fs, delta 1, in process cycle_fretc.reg",
        ),
        // The check of the target, after the call returned: the place is the statement's.
        (
            "cycle_fback",
            "error[E3009]: value 9 outside of INTEGER range 0 to 7 for signal X",
            "at 0 fs, delta 1, in process cycle_fback.reg",
        ),
        // A part of an element at the index just before the array.
        (
            "cycle_eslice",
            "error[E3009]: index -1 outside of range 0 to 3 for v",
            "at 30 ns, delta 1, in process cycle_eslice.reg",
        ),
    ] {
        let mut shown = Vec::new();
        for mode in [None, Some("--cycle")] {
            let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
            args.extend(mode);
            args.push(top);
            let o = yavs(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "{top} {mode:?}: {err}");
            assert!(
                err.contains(msg) && err.contains(note),
                "{top} {mode:?}: {err}"
            );
            shown.push(err[err.find("error[").expect("an error")..].to_string());
            if mode.is_some() {
                assert!(
                    err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                    "{err}"
                );
            }
        }
        // The same diagnostic, with the place of the statement and of the calls around it.
        assert_eq!(shown[0], shown[1], "{top}");
    }
}

/// A call whose constant actual lies outside its formal's subtype stays in the event kernel,
/// which fails the check as the normal mode does.
#[test]
fn a_constant_actual_outside_its_formal_keeps_the_call_out() {
    let dir = tempdir("cycle-call-const");
    analyze(&dir, "checks.vhd", CALL_CHECKS);
    for mode in [None, Some("--cycle")] {
        let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
        args.extend(mode);
        args.push("cycle_fparc");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{mode:?}: {err}");
        assert!(
            err.contains("value -1 outside of NATURAL range 0 to 2147483647 for parameter V")
                && err.contains("at 0 fs, delta 1, in process cycle_fparc.reg"),
            "{mode:?}: {err}"
        );
        if mode.is_some() {
            assert!(
                err.contains("cycle mode: 0 of 2 processes run cycle-based"),
                "{err}"
            );
        }
    }
}

/// A `'Z'` made inside a called function keeps its caller out, with the reason.
#[test]
fn a_high_impedance_value_made_in_a_called_function_keeps_the_caller_out() {
    let dir = tempdir("cycle-call-z");
    analyze(&dir, "z.vhd", Z_CALL);
    let (normal, _) = run(&dir, &["--tier=interp"], "cycle_fz");
    let (cycle, err) = run(&dir, &["--tier=interp", "--cycle"], "cycle_fz");
    assert_eq!(normal, ["** Note: 5000000fs+0: o='Z'"], "{normal:?}");
    assert_eq!(cycle, normal);
    assert!(
        err.contains("cycle mode: 0 of 2 processes run cycle-based"),
        "{err}"
    );
    assert!(
        err.contains("cycle_fz.o <= ... (z.vhd:")
            && err.contains("a `'Z'` literal: two-state values have no high impedance"),
        "{err}"
    );
}

const Z_CALL: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_fz is
end entity;
architecture a of cycle_fz is
  signal a, o : std_ulogic := '0';
  function pick(x : std_ulogic) return std_ulogic is
  begin
    if x = '1' then
      return 'Z';
    end if;
    return '0';
  end function;
begin
  o <= pick(a);
  main : process
  begin
    a <= '1';
    wait for 5 ns;
    report \"o=\" & std_ulogic'image(o);
    wait;
  end process;
end architecture;
";

/// `src` with the token `W` replaced by `w`.
fn with_width(src: &str, w: u32) -> String {
    let mut t = String::new();
    let b = src.as_bytes();
    for (i, c) in src.char_indices() {
        let word = |k: Option<&u8>| k.is_some_and(|x| x.is_ascii_alphanumeric() || *x == b'_');
        if c == 'W' && !word(i.checked_sub(1).and_then(|k| b.get(k))) && !word(b.get(i + 1)) {
            t.push_str(&w.to_string());
        } else {
            t.push(c);
        }
    }
    t
}

/// `numeric.vhd` at width `w`: the token `W` replaced, the operations of one word (shifts by a
/// run-time count, division) and the integer conversions where the width allows them.
fn numeric_design(w: u32) -> String {
    let t = with_width(include_str!("cycle/numeric.vhd"), w);
    let one_word = w <= 64;
    let dyn_ = "    slk <= shift_left(unsigned(a), k);
    srk <= shift_right(unsigned(a), k);
    srak <= unsigned(shift_right(signed(a), k));
    rlk <= rotate_left(unsigned(a), k);
    rrk <= rotate_right(unsigned(a), k);";
    let mut div = String::from(
        "  divs : process (all)
  begin
    if unsigned(b) /= 0 then
      q_div <= unsigned(a) / unsigned(b);
      q_mod <= unsigned(a) mod unsigned(b);
      q_rem <= unsigned(a) rem unsigned(b);
      s_div <= unsigned(signed(a) / signed(b));
      s_mod <= unsigned(signed(a) mod signed(b));
      s_rem <= unsigned(signed(a) rem signed(b));
    else
      q_div <= (others => '0');
      q_mod <= (others => '0');
      q_rem <= (others => '0');
      s_div <= (others => '0');
      s_mod <= (others => '0');
      s_rem <= (others => '0');
    end if;
  end process;",
    );
    let mut print = String::from(
        "& \" slk=\" & to_hstring(slk) & \" srk=\" & to_hstring(srk) & \" srak=\" & to_hstring(srak)
        & \" rlk=\" & to_hstring(rlk) & \" rrk=\" & to_hstring(rrk) & \" qd=\" & to_hstring(q_div)
        & \" qm=\" & to_hstring(q_mod) & \" qr=\" & to_hstring(q_rem) & \" sd=\" & to_hstring(s_div)
        & \" sm=\" & to_hstring(s_mod) & \" sr=\" & to_hstring(s_rem)",
    );
    if w <= 31 {
        div.push_str(&format!(
            "
  toi : process (all)
  begin
    ti <= to_integer(unsigned(a)) - to_integer(signed(b));
  end process;
  tou : process (all)
  begin
    tu <= to_unsigned(abs ti, {w}) xor unsigned(to_signed(ti, {w}));
  end process;"
        ));
        print.push_str("\n        & \" ti=\" & integer'image(ti) & \" tu=\" & to_hstring(tu)");
    }
    if !one_word {
        (div, print) = (String::new(), String::new());
    }
    t.replace("--DYN", if one_word { dyn_ } else { "" })
        .replace("--DIV", &div)
        .replace("--PRINT", &print)
}

#[test]
fn numeric_operations_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-numeric");
    for w in [1, 63, 64, 65, 128, 200] {
        analyze(&dir, &format!("numeric{w}.vhd"), &numeric_design(w));
        // `arith`, `cmps`, `shifts`, `reds`, `fsm` and the selected assignment, then `divs`
        // (one word) and `toi`, `tou` (an integer of the width).
        let members = match w {
            ..=31 => "9 of 10",
            32..=64 => "7 of 8",
            _ => "6 of 7",
        };
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_ops2");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_ops2");
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                "{w}: {err}"
            );
        }
        // One word in registers at a time: every operation's words through memory, and the
        // temporaries no later operation reads dropped unstored (`YAVS_CYCLE_REGS`).
        let (normal, _) = run(&dir, &["--tier=interp"], "cycle_ops2");
        let flags = ["--tier=interp", "--cycle"];
        for cap in ["1", "2"] {
            let (cycle, err) = run_vars(&dir, &flags, "cycle_ops2", &[("YAVS_CYCLE_REGS", cap)]);
            assert_eq!(cycle, normal, "width {w}, {cap} words in registers\n{err}");
        }
    }
}

#[test]
fn subprogram_calls_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-calls");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("calls{w}.vhd"),
            &with_width(include_str!("cycle/calls.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_calls");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_calls");
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main`.
            assert!(
                err.contains("cycle mode: 6 of 7 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

#[test]
fn arrays_of_records_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-recarr");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("recarr{w}.vhd"),
            &with_width(include_str!("cycle/recarr.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_recarr");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_recarr");
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main_p`.
            assert!(
                err.contains("cycle mode: 7 of 8 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A 'Z' copied by a process that stays out for another reason (a delay) reaches a register the
/// testbench reads: the register stays out too and the testbench sees the 'Z'; a register of a
/// line without one joins, and so do one of a line with a pull-up (it never resolves to 'Z') and
/// one of a copy outside that only tests the line; copies through a variable, `to_x01` and
/// `'delayed`, and a line pulled up on one element only, keep theirs out. Against the normal mode
/// at widths 1 to 128.
#[test]
fn a_z_passed_on_by_a_process_outside_keeps_its_reader_out() {
    let dir = tempdir("cycle-zpass");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("zpass{w}.vhd"),
            &with_width(include_str!("cycle/zpass.vhd"), w),
        );
        for t in tiers() {
            // Width 1 truncates `k + 3`: the warnings name `clean <= ...`, not a result line.
            let k = |v: Vec<String>| -> Vec<String> {
                v.into_iter().filter(|l| l.contains("k=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_zpass");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_zpass");
            let (normal, cycle) = (k(normal), k(cycle));
            assert_eq!(normal.len(), 8, "{w} {t:?} {normal:?}");
            assert!(normal.iter().any(|l| l.contains("q=Z")), "{normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 20 processes run cycle-based"),
                "{w}: {err}"
            );
            // The members, listed as clocked.
            for m in ["reg_r", "reg_p", "reg_c"] {
                let row = format!("cycle_zpass.{m} (zpass");
                assert!(
                    err.lines()
                        .any(|l| l.contains(&row) && l.trim_end().ends_with("clocked")),
                    "{err}"
                );
            }
        }
    }
}

/// Records holding arrays of records (register-file converters): an inlined function's local of
/// such a record, filled field by field, one element copied from a run-time index, and a local
/// with no initial value at its fields' `'left`; against the normal mode at widths 1 to 128.
#[test]
fn records_holding_arrays_of_records_equal_the_normal_mode() {
    let dir = tempdir("cycle-recarrrec");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("recarrrec{w}.vhd"),
            &with_width(include_str!("cycle/recarrrec.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_recarrrec");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_recarrrec");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // `conv` and `other <= ...`; the testbench's `main` stays out.
            assert!(
                err.contains("cycle mode: 2 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A logic scalar converted to its own type (`std_logic(x)`, `std_ulogic(x)`) joins; converted to
/// a range of the type (`X01`), which checks the value, its process stays out. Against the normal
/// mode at widths 1 to 128.
#[test]
fn logic_scalar_conversions_equal_the_normal_mode() {
    let dir = tempdir("cycle-logicconv");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("logicconv{w}.vhd"),
            &with_width(include_str!("cycle/logicconv.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_logicconv");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_logicconv");
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 6 processes run cycle-based"),
                "{w}: {err}"
            );
            assert!(err.contains("cycle_logicconv.checked <= ..."), "{err}");
        }
    }
}

/// Slices whose bounds come from variables: known values fold (a loop parameter through two
/// variables, both branches, a `case`, an `inout` formal, a function's locals), unknown ones
/// do not (branches that differ, a loop that may exit, a state variable), and two variables
/// chosen at run time keep their process out instead of being taken for a one-element slice,
/// which failed at run time with a false length error. An initial value is evaluated where the
/// run starts, not where the body first names its variable.
#[test]
fn slices_with_bounds_from_variables_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-slices");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("slices{w}.vhd"),
            &with_width(include_str!("cycle/slices.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_slices");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_slices");
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main` and `apart`.
            assert!(
                err.contains("cycle mode: 7 of 9 processes run cycle-based"),
                "{w}: {err}"
            );
            let (_, why) = row(&err, "cycle_slices.apart");
            assert!(
                why.contains("a slice whose length is not known during elaboration"),
                "{w}: {err}"
            );
        }
    }
}

/// D14 (outside processes see a region output one delta after the trigger, whatever the logic in
/// between): a reader one delta after the edge (`wait for 0 ns`) sees the register's value through
/// a concurrent copy one edge earlier than in the normal mode, which takes a delta per process.
/// Pinned so a change of the contract shows; vhdl-ai-test's `test_reset_mid_pass` leaves its wait
/// for idle one clock earlier for this reason.
const DELTA_READER: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_d14 is end entity;
architecture a of cycle_d14 is
  signal clk, reset, q, idle : std_ulogic := '0';
begin
  clk <= not clk after 5 ns when now < 100 ns;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= reset;
    end if;
  end process;
  idle <= q;
  main : process
  begin
    for c in 1 to 4 loop
      reset <= '1' when c = 2 else '0';
      wait until rising_edge(clk);
      wait for 0 ns;
      report \"c=\" & integer'image(c) & \" idle=\" & std_ulogic'image(idle);
    end loop;
    wait;
  end process;
end architecture;
";

#[test]
fn a_reader_one_delta_after_the_edge_sees_region_logic_settled() {
    let dir = tempdir("cycle-d14");
    analyze(&dir, "d14.vhd", DELTA_READER);
    let at = |lines: &[String]| {
        (lines.iter())
            .position(|l| l.ends_with("idle='1'"))
            .map(|k| k + 1)
    };
    let (normal, _) = run(&dir, &["--tier=interp"], "cycle_d14");
    let (cycle, err) = run(&dir, &["--tier=interp", "--cycle"], "cycle_d14");
    assert_eq!(at(&normal), Some(3), "{normal:?}");
    assert_eq!(at(&cycle), Some(2), "{cycle:?}\n{err}");
    assert!(
        err.contains("cycle mode: 2 of 4 processes run cycle-based"),
        "{err}"
    );
}

/// A literal aggregate of an array of records (a reset table whose elements differ, nested
/// records, booleans) as the record of arrays the kernel keeps (`tests/cycle/reclit.vhd`).
#[test]
fn literals_of_arrays_of_records_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-reclit");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("reclit{w}.vhd"),
            &with_width(include_str!("cycle/reclit.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_reclit");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_reclit");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Signals of arrays of integers and enumerations: the kernel keeps their elements from the
/// left, the image from the right. Driven outside and read by members (by element, at a run-time
/// index), an array of arrays through next-state blocks, a memory at run-time indices, a port
/// collapsed onto a slice, 8-byte integers and enumerations in and out; 1 to 129 elements.
#[test]
fn signals_of_integer_arrays_equal_the_normal_mode() {
    let dir = tempdir("cycle-intsig");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("intsig{w}.vhd"),
            &with_width(include_str!("cycle/intsig.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_intsig");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_intsig");
            assert_eq!(normal.len(), 20, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main`.
            assert!(
                err.contains("cycle mode: 6 of 7 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Constants a plain elaboration evaluates only when something needs it (an architecture
/// constant computed by a function, a deferred package constant), read only by region code.
#[test]
fn constants_only_region_code_reads_have_their_values() {
    let dir = tempdir("cycle-consts");
    analyze(&dir, "consts.vhd", include_str!("cycle/consts.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_consts");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_consts");
        assert_eq!(normal.len(), 4, "{t:?} {normal:?}");
        assert!(normal[0].ends_with("o=ED h=38"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 1 of 2 processes run cycle-based"),
            "{err}"
        );
    }
}

/// `src` at width `w` with its string literals `PAT1` to `PAT3` of `w` elements filled in.
fn with_pats(src: &str, w: u32) -> String {
    let mut t = with_width(src, w);
    for k in 1..=3 {
        let lit: String = (0..w)
            .map(|i| {
                if (i * i + i * k + k) % 5 < 2 {
                    '1'
                } else {
                    '0'
                }
            })
            .collect();
        t = t.replace(&format!("PAT{k}"), &format!("\"{lit}\""));
    }
    t
}

/// Bits, slices and elements of constant arrays of vectors: an element has its subtype's index
/// range, not the `0 to n - 1` of the literal or function result that made it (also in an
/// aggregate given to a formal or returned). Read by its literal's range, a `downto` element came
/// out reversed and a range not starting at 0 failed.
#[test]
fn elements_of_constant_arrays_have_their_subtypes_ranges_at_widths_around_a_word() {
    let dir = tempdir("cycle-constelem");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("constelem{w}.vhd"),
            &with_pats(include_str!("cycle/constelem.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_constelem");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_constelem");
            assert_eq!(normal.len(), 7, "{w} {t:?} {normal:?}");
            assert!(normal[6].contains(" s8=9A01 "), "{normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the clock and the testbench's `main`.
            assert!(
                err.contains("cycle mode: 9 of 11 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A signal a clocked process assigns itself after an earlier assignment of all or part of it in
/// the same run (a vector, a record, a memory, an integer) keeps its value. The region had taken
/// `q <= q` for a no-op, so the earlier assignment stood (found by the fuzzer).
#[test]
fn a_signal_assigned_itself_after_an_earlier_assignment_keeps_its_value() {
    let dir = tempdir("cycle-selfassign");
    analyze(&dir, "selfassign.vhd", include_str!("cycle/selfassign.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_selfassign");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_selfassign");
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A logical right shift by a count known during elaboration (of 1, 63, 64, 65, 100 and the width
/// less one) gives zeros above the bits that remain, also into a signal or variable that held
/// another value in the same run: the region wrote only the words of the remaining bits.
#[test]
fn a_right_shift_by_a_known_count_writes_every_word_of_its_target() {
    let dir = tempdir("cycle-shiftfill");
    analyze(&dir, "shiftfill.vhd", include_str!("cycle/shiftfill.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_shiftfill");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_shiftfill");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 4 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Consecutive `if`s whose conditions are computed the same way join behind one branch: a register
/// file written and read bit by bit under `if idx = k`; variables updated in place before each `if`
/// (`a := a and en`) look alike but read other values and stay apart (they joined at first).
#[test]
fn ifs_on_one_condition_join_and_look_alike_ones_do_not() {
    let dir = tempdir("cycle-joinif");
    analyze(&dir, "joinif.vhd", include_str!("cycle/joinif.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_joinif");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_joinif");
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 5 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Globally static calls on bits and slices of elements of constant arrays, folded from
/// elaboration's values: an element has its subtype's index range, also when bit-string literals
/// (`0 to n - 1`) gave it. Elaboration had read them by the literal's range (found by the fuzzer).
#[test]
fn calls_on_elements_of_constant_tables_fold_by_the_elements_ranges() {
    let dir = tempdir("cycle-constcall");
    analyze(&dir, "constcall.vhd", include_str!("cycle/constcall.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_constcall");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_constcall");
        assert_eq!(normal.len(), 6, "{t:?} {normal:?}");
        assert!(
            normal[0].contains("q1='1' q2='1' q3=B q4=4 c1='1' c2='0' c3=9"),
            "{normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 4 of 6 processes run cycle-based"),
            "{err}"
        );
    }
}

/// A variable given an integer just computed and then assigned to the element of an array of one
/// integer keeps its value: the element's word was taken for a fresh value, and the operation that
/// wrote the variable was made to write the element instead.
#[test]
fn a_variable_given_to_an_array_of_one_integer_keeps_its_value() {
    let dir = tempdir("cycle-intelem1");
    analyze(&dir, "intelem1.vhd", include_str!("cycle/intelem1.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_intelem1");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_intelem1");
        assert_eq!(normal.len(), 6, "{t:?} {normal:?}");
        assert!(normal[0].contains("q=8 q2=5 q3=19 s=5"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 4 processes run cycle-based"),
            "{err}"
        );
    }
}

/// A register reset asynchronously by another register changes twice in one time step in the
/// normal mode; a process below the top whose list names it alone and that reads logic following
/// it sees the value between. The register stays out with that logic (it joined, and the
/// process's output differed).
#[test]
fn a_register_with_an_internal_reset_feeding_a_short_list_stays_out() {
    let dir = tempdir("cycle-runreset");
    analyze(&dir, "runreset.vhd", include_str!("cycle/runreset.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_runreset");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_runreset");
        assert_eq!(normal.len(), 12, "{t:?} {normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 7 processes run cycle-based")
                && err.contains(
                    "it computes, as a register with an asynchronous reset, cycle_runreset.q, in \
                     the fan-in of cycle_runreset.dut.list"
                ),
            "{err}"
        );
    }
}

/// A register reset asynchronously by another register pulses for one delta in the normal mode
/// when the reset rises at an edge that changes it; the pulse resets and clocks other registers.
/// The region settled edge and reset together and showed no pulse, so that register stays out.
#[test]
fn a_register_whose_pulse_resets_or_clocks_another_stays_out() {
    let dir = tempdir("cycle-pulse");
    analyze(&dir, "pulse.vhd", include_str!("cycle/pulse.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_pulse");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_pulse");
        assert_eq!(normal.len(), 12, "{t:?} {normal:?}");
        assert!(normal[2].contains("t=00"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 6 processes run cycle-based")
                && err.contains("its asynchronous reset cycle_pulse.r is made by a register"),
            "{err}"
        );
    }
}

/// Arrays of integers and enumerations in variables, constants and signals.
#[test]
fn arrays_of_integers_give_the_same_result() {
    let dir = tempdir("cycle-intarr");
    analyze(&dir, "intarr.vhd", include_str!("cycle/intarr.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_intarr");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_intarr");
        assert_eq!(normal.len(), 16, "{t:?} {normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        // Every process but the testbench's `main`.
        assert!(
            err.contains("cycle mode: 7 of 8 processes run cycle-based"),
            "{err}"
        );
    }
}

/// Memories written by clocked members at run-time indices: their writes wait for the commit, so a
/// read at the same edge gives the old value (`tests/cycle/ram.vhd`); one the testbench reads is
/// exported by the words its writes touched.
#[test]
fn memories_equal_the_normal_mode_at_widths_around_a_word() {
    let dir = tempdir("cycle-ram");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("ram{w}.vhd"),
            &with_width(include_str!("cycle/ram.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_ram");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_ram");
            assert_eq!(normal.len(), 20, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the clock and the testbench's `main`.
            assert!(
                err.contains("cycle mode: 5 of 7 processes run cycle-based"),
                "{w}: {err}"
            );
            let (section, _) = row(&err, "cycle_ram.shadow");
            assert_eq!(section, "cycle-based:", "{w}: {err}");
        }
    }
}

#[test]
fn clocked_and_combinational_shapes_give_the_same_result() {
    let dir = tempdir("cycle-shapes");
    analyze(&dir, "shapes.vhd", include_str!("cycle/shapes.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_shapes");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_shapes");
        // Hand-checked against the normal mode: the resets at cycles 3 and 17 clear `acc`.
        assert_eq!(normal.len(), 40, "{t:?} {normal:?}");
        assert!(normal[2].contains("acc=00 cnt=0"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 14 of 15 processes run cycle-based"),
            "{err}"
        );
    }
}

/// Standard output of a successful run with the delta numbers of report stamps left out (a region
/// output changes one delta after its trigger, D14).
fn run_text(dir: &Path, flags: &[&str], top: &str) -> (String, String) {
    let mut args = vec!["run", "--work=work:lib", "--no-cache"];
    args.extend(flags);
    args.push(top);
    let o = yavs(dir, &args);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(o.status.success(), "{top} {flags:?}: {out}{err}");
    let err = with_report(dir, flags, top, err);
    let mut text = String::new();
    for l in out.lines() {
        match l.split_once("fs+") {
            Some((a, b)) => {
                text.push_str(a);
                text.push_str("fs");
                text.push_str(b.trim_start_matches(|c: char| c.is_ascii_digit()));
            }
            None => text.push_str(l),
        }
        text.push('\n');
    }
    (text, err)
}

/// The RTL designs of the benchmark set, shortened: the same output in cycle mode, with their RTL
/// processes inside the region. `lfsr64` prints its checksum from a process that joins (whose
/// reports cycle mode does not evaluate), so the test moves the report into a process of its own.
#[test]
fn bench_rtl_designs_give_the_same_output_in_cycle_mode() {
    let lfsr = |src: &str| {
        src.replace("report \"sum=\" & integer'image(to_integer(a));", "null;")
            .replace(
                "end architecture;",
                "  show : process\n  begin\n    wait until done;\n    \
             report \"sum=\" & integer'image(to_integer(sums));\n    wait;\n  end process;\n\
             end architecture;",
            )
    };
    let designs: [(&str, String, &str, &[&str], &str); 7] = [
        (
            "rtl_ff",
            include_str!("../../../tools/bench/designs/rtl_ff.vhd").into(),
            "bench_rtl_ff",
            &["-gN=256", "-gCYC=40"],
            "256 of 258",
        ),
        (
            "rtl_chain",
            include_str!("../../../tools/bench/designs/rtl_chain.vhd").into(),
            "bench_rtl_chain",
            &["-gN=100", "-gCYC=40"],
            "102 of 104",
        ),
        (
            "rtl_fan",
            include_str!("../../../tools/bench/designs/rtl_fan.vhd").into(),
            "bench_rtl_fan",
            &["-gN=100", "-gCYC=40"],
            "100 of 102",
        ),
        (
            "ff_rtl",
            include_str!("../../../tools/bench/designs/ff_rtl.vhd").into(),
            "bench_ff_rtl",
            &["-gLANES=8", "-gCYCLES=40"],
            "8 of 10",
        ),
        (
            "lfsr64",
            lfsr(include_str!("../../../tools/bench/designs/lfsr64.vhd")),
            "bench_lfsr64",
            &["-gNREG=8", "-gCYCLES=200"],
            "9 of 11",
        ),
        (
            "lfsr64_xor",
            lfsr(include_str!("../../../tools/bench/designs/lfsr64_xor.vhd")),
            "bench_lfsr64_xor",
            &["-gNREG=8", "-gCYCLES=200"],
            "9 of 11",
        ),
        // A memory of 65536 elements: its writes wait for the commit.
        (
            "rtl_ram",
            include_str!("../../../tools/bench/designs/rtl_ram.vhd").into(),
            "bench_rtl_ram",
            &["-gCYC=40"],
            "1 of 3",
        ),
    ];
    for (name, src, top, generics, members) in designs {
        let dir = tempdir(&format!("cycle-bench-{name}"));
        analyze(&dir, &format!("{name}.vhd"), &src);
        for t in tiers() {
            let flags = [&t[..], generics].concat();
            let (normal, _) = run_text(&dir, &flags, top);
            let (cycle, err) = run_text(&dir, &[&flags[..], &["--cycle"]].concat(), top);
            assert!(normal.contains("Note"), "{name}: {normal}");
            assert_eq!(cycle, normal, "{name} {t:?}\n{err}");
            assert!(
                err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                "{name}: {err}"
            );
        }
    }
}

/// Bench designs whose processes cannot join (yet), with the reason the report gives.
#[test]
fn bench_designs_cycle_mode_leaves_out_say_why() {
    for (name, src, top, why) in [
        (
            "rtl_lfsr",
            include_str!("../../../tools/bench/designs/rtl_lfsr.vhd"),
            "bench_rtl_lfsr",
            "a comparison with a metavalue literal",
        ),
        (
            "rtl_bus",
            include_str!("../../../tools/bench/designs/rtl_bus.vhd"),
            "bench_rtl_bus",
            "E4118 a `'Z'` value",
        ),
    ] {
        let dir = tempdir(&format!("cycle-out-{name}"));
        analyze(&dir, &format!("{name}.vhd"), src);
        let flags = ["--tier=interp", "-gCYC=20"];
        let (normal, _) = run_text(&dir, &flags, top);
        let (cycle, err) = run_text(&dir, &[&flags[..], &["--cycle"]].concat(), top);
        assert_eq!(cycle, normal, "{name}\n{err}");
        assert!(err.contains(" 0 of "), "{name}: {err}");
        assert!(err.contains(why), "{name}: {err}");
    }
}

/// A member that drives a null array (a pipeline of no stages under these generics): the region
/// has nothing to read or drive of it (it once failed at initialisation with an invalid reference).
const NULL_PIPE: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_null is
  generic (STAGES : natural := 0);
end entity;
architecture a of cycle_null is
  type pipe_t is array (1 to STAGES) of std_ulogic_vector(3 downto 0);
  signal p : pipe_t;
  signal clk : std_ulogic := '0';
  signal q : std_ulogic_vector(3 downto 0) := \"0101\";
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      for k in 1 to STAGES loop
        p(k) <= q;
      end loop;
      q <= q(2 downto 0) & not q(3);
    end if;
  end process;
  main : process
  begin
    for i in 1 to 5 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report \"q=\" & to_string(q);
    wait;
  end process;
end architecture;
";

/// The verdicts of the subset checker on a design, without running it.
fn verdicts(dir: &Path, top: &str) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-report",
            top,
        ])
        .output()
        .expect("run yavs");
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(o.status.success(), "{err}");
    assert!(o.stdout.is_empty(), "nothing runs: {err}");
    err
}

/// The report row of process `label`, and the section it is in: the heading above it, ending
/// with a colon, as the sections of the report read before it was grouped by reason.
fn row<'a>(report: &'a str, label: &str) -> (&'a str, &'a str) {
    let mut section = "";
    for l in report.lines() {
        if l.ends_with(':') {
            section = match l.trim() {
                h if h.starts_with("cycle-based") => "cycle-based:",
                h if h.starts_with("behavioural") => "behavioural, in the event kernel:",
                h if h.starts_with("E4101") => "in the subset, not implemented in cycle mode yet:",
                h if h.starts_with('E') => "outside the subset, in the event kernel:",
                h => h,
            };
        }
        if l.trim_start().starts_with(&format!("{label} (")) {
            return (section, l);
        }
    }
    panic!("no row for {label}:\n{report}");
}

/// The line of `subset.vhd` holding `needle`, at or after the line of process `label`.
fn line_of(label: &str, needle: &str) -> usize {
    let text = include_str!("cycle/subset.vhd");
    let lines: Vec<&str> = text.lines().collect();
    let start = (lines.iter())
        .position(|l| l.trim_start().starts_with(&format!("{label} :")))
        .unwrap_or_else(|| panic!("no process {label}"));
    let at = (lines[start..].iter())
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no `{needle}` in {label}"));
    start + at + 1
}

#[test]
fn subset_reasons_are_reported_with_their_code_and_position() {
    let dir = tempdir("cycle-subset");
    analyze(&dir, "subset.vhd", include_str!("cycle/subset.vhd"));
    let report = verdicts(&dir, "sub_tb");
    // Process, code, the construct named (its line is the position given).
    let want = [
        ("wait_on", "E4110", "wait on a;"),
        ("delayed", "E4111", "after 1 ns"),
        ("history", "E4112", "'last_value"),
        ("real_var", "E4113", "variable acc"),
        ("shared_call", "E4114", "sh.inc"),
        ("late", "E4115", "postponed"),
        ("changes_state", "E4117", "ri <= bump"),
        ("high_z", "E4118", "'Z'"),
        ("latch", "E4119", "if en = '1'"),
        ("state", "E4120", "q7 <= v"),
        ("incomplete", "E4121", "q8 <= a and b"),
        ("loop_x", "E4122", "loop_x"),
        ("loop_y", "E4122", "loop_y"),
        ("outside_edge", "E4123", "rising_edge"),
        ("drive_1", "E4124", "drive_1"),
        ("drive_2", "E4124", "drive_2"),
        ("gate", "E4125", "gate"),
        ("on_divided", "E4125", "on_divided"),
        ("external", "E4126", "<<signal"),
        ("real_read", "E4113", "ri2 <="),
        ("before_wait", "E4123", "g1 <= a"),
        ("clock_unlisted", "E4123", "rising_edge"),
        ("reset_unlisted", "E4123", "if rst = '1'"),
        ("reset_reads", "E4121", "if rst = '1'"),
        ("on_without_clock", "E4123", "wait on a until"),
        // Assigned only when the loop was not left early: a loop through the exit condition.
        ("early_exit", "E4122", "early_exit"),
        ("from_w", "E4122", "from_w"),
        ("called_history", "E4112", "prev(a)"),
        // The static compare of an enumeration parameter keeps the edge from `e2`.
        ("enum_loop", "E4122", "enum_loop"),
        ("from_e1", "E4122", "from_e1"),
        // A loop past the budget: no pruning. (`give_up`'s function, which the checker's own
        // evaluation gives up on, is evaluated by elaboration: its branch is pruned.)
        ("long_loop", "E4119", "if i = 99999"),
        ("transport_delay", "E4111", "<= transport"),
        ("reject_limit", "E4111", "<= reject"),
        ("waveform", "E4111", "after 2 ns"),
        ("guarding.h6 <= ...", "E4115", "guarded a"),
        ("guarding.GUARD <= ...", "E4115", "block (en"),
        ("calls_foreign", "E4117", "outside(a)"),
        ("with_else", "E4123", "rising_edge"),
        ("nested_wait", "E4123", "wait until"),
        ("slow_loop", "E4126", "while"),
        ("drives_bus", "E4124", "drives_bus"),
        ("drives_resolved", "E4124", "drives_resolved"),
    ];
    for (label, code, needle) in want {
        let line = line_of(label.split('.').next().expect("a label"), needle);
        let (section, r) = row(&report, &format!("sub_tb.dut.{label}"));
        assert_eq!(section, "outside the subset, in the event kernel:", "{r}");
        assert!(
            r.contains(&format!("  {code} ")) && r.ends_with(&format!("(subset.vhd:{line})")),
            "{label}, want {code} at line {line}: {r}"
        );
    }
    // A port on a slice of its actual: the latch is named in the actual's elements.
    {
        let (section, r) = row(&report, "sub_tb.lower.bad_bits.bits");
        let line = line_of("bad_bits", "if x(0) = '1'");
        assert_eq!(section, "outside the subset, in the event kernel:", "{r}");
        assert!(
            r.contains("E4119 it does not assign sub_tb.bus16(7) on every path")
                && r.ends_with(&format!("(subset.vhd:{line})")),
            "{r}"
        );
    }
    // Loops through record ports associated field by field: a port edge the kernel updates, and
    // a port collapsed into its actual's leaf (whose record path is the actual's, not the
    // formal's: by position).
    for label in ["fields.pass", "back", "fields.pass_one", "back_one"] {
        let (section, r) = row(&report, &format!("sub_tb.{label}"));
        assert_eq!(section, "outside the subset, in the event kernel:", "{r}");
        assert!(r.contains("  E4122 it is in a combinational loop"), "{r}");
    }
    assert!(
        report.contains("sub_tb.gx -> sub_tb.fields.pass_one -> sub_tb.go -> sub_tb.back_one")
            && report.contains("sub_tb.fx -> a port -> sub_tb.fields.i.a -> sub_tb.fields.pass"),
        "{report}"
    );
    // The loop is named element by element, through both processes.
    assert!(
        report.contains(
            "sub_tb.dut.x -> sub_tb.dut.loop_y -> sub_tb.dut.y -> sub_tb.dut.loop_x -> sub_tb.dut.x"
        ),
        "{report}"
    );
    // Testbench kinds stay in the event kernel by nature, with the reason.
    for (label, why) in [
        ("timed", "it waits for a time"),
        ("forever", "it waits forever (`wait;`)"),
        ("two_waits", "it has 2 wait statements"),
        ("logger", "it declares `l`, an access value"),
        ("waits_inside", "it calls `tick`, which may wait"),
        ("dut.protected_var", "it declares `cnt`, a protected object"),
    ] {
        let (section, r) = row(&report, &format!("sub_tb.{label}"));
        assert_eq!(section, "behavioural, in the event kernel:", "{r}");
        assert!(r.contains(why), "{r}");
    }
}

#[test]
fn every_shape_of_the_subset_fits() {
    let dir = tempdir("cycle-shapes");
    analyze(&dir, "subset.vhd", include_str!("cycle/subset.vhd"));
    let report = verdicts(&dir, "sub_tb");
    for label in [
        "clocked",
        "falling",
        "legacy",
        "enabled",
        "wait_edge",
        "wait_enabled",
        "async",
        "async_two",
        "comb_all",
        "comb_if",
        "f11 <= ...",
        "f12 <= ...",
        "f13 <= ...",
        "f14 <= ...",
        "assert",
        "levels",
        "comb_rec",
        "reg_rec",
        "reset_maker",
        "made_reset",
        "on_gated",
        "enum_only",
        "divider",
        "static_call",
        "clocked_proc",
        "partial_var",
    ] {
        let (section, r) = row(&report, &format!("sub_tb.dut.{label}"));
        assert!(
            section == "cycle-based:"
                || section == "in the subset, not implemented in cycle mode yet:",
            "{label} does not fit: {r}"
        );
    }
    // Its writes map to the actual's elements from the slice's offset, as its drivers do.
    let (section, r) = row(&report, "sub_tb.upper.good_bits.bits");
    assert!(
        section.starts_with("in the subset") || section == "cycle-based:",
        "{r}"
    );
    // `levels` reads its own levels: no loop of elements, three ordered passes (or a split).
    assert!(
        report.contains("subset: 28 of 81 processes fit, 1 loop group among them")
            && report.contains(
                "loop group of 1 process: 3 passes, or 3 pieces after a split: sub_tb.dut.levels"
            ),
        "{report}"
    );
    // Asserts and reports are counted, never a reason to stay out (the other holder computes
    // logic a process with a foreign call may read: the run-count rule keeps it out).
    assert!(
        report.contains("1 assert and reports not evaluated in cycle mode, held by 1 cycle-based"),
        "{report}"
    );
}

#[test]
fn force_is_outside_the_subset() {
    let dir = tempdir("cycle-force");
    analyze(
        &dir,
        "force.vhd",
        "library ieee;\nuse ieee.std_logic_1164.all;\nentity cycle_force is end entity;\n\
         architecture a of cycle_force is\n  signal a, q : std_ulogic := '0';\nbegin\n\
         \x20 f : process (a)\n  begin\n    q <= force a;\n  end process;\nend architecture;\n",
    );
    let report = verdicts(&dir, "cycle_force");
    let (_, r) = row(&report, "cycle_force.f");
    assert!(r.contains("E4116 ") && r.ends_with("(force.vhd:9)"), "{r}");
}

/// Processes that would give another result if cycle mode took them: they stay out, and the
/// run equals the normal mode.
const WRONG_IF_TAKEN: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_wrong is
end entity;
architecture a of cycle_wrong is
  signal a1, b1, a2, en, a3 : std_ulogic := '0';
  signal q_list, q_latch, q_state : std_ulogic := '0';
begin
  -- Runs on `a1` only: after `b1` changes alone, the normal mode keeps the old value.
  list : process (a1)
  begin
    q_list <= a1 and b1;
  end process;
  -- Keeps its value while `en` is low.
  latch : process (a2, en)
  begin
    if en = '1' then
      q_latch <= a2;
    end if;
  end process;
  -- Gives the value of `a3` from its previous run.
  state : process (a3)
    variable v : std_ulogic := '0';
  begin
    q_state <= v;
    v := a3;
  end process;
  main : process
  begin
    a1 <= '1';
    a2 <= '1';
    a3 <= '1';
    wait for 1 ns;
    b1 <= '1';
    en <= '1';
    wait for 1 ns;
    en <= '0';
    a2 <= '0';
    a3 <= '0';
    wait for 1 ns;
    report \"q_list=\" & to_string(q_list) & \" q_latch=\" & to_string(q_latch)
      & \" q_state=\" & to_string(q_state);
    wait;
  end process;
end architecture;
";

#[test]
fn a_member_driving_a_null_array_gives_the_same_result() {
    let dir = tempdir("cycle-null");
    analyze(&dir, "null.vhd", NULL_PIPE);
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_null");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_null");
        assert!(
            normal.len() == 1 && normal[0].ends_with("q=0100"),
            "{normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 1 of 2 processes run cycle-based"),
            "{err}"
        );
    }
}

#[test]
fn latches_state_and_incomplete_lists_stay_out_and_the_result_is_the_normal_one() {
    let dir = tempdir("cycle-wrong");
    analyze(&dir, "wrong.vhd", WRONG_IF_TAKEN);
    // Hand-checked: functions of the inputs alone would give q_list=1 q_latch=0 q_state=0.
    let (normal, _) = run(&dir, &["--tier=interp"], "cycle_wrong");
    assert!(
        normal.len() == 1 && normal[0].ends_with("q_list=0 q_latch=1 q_state=1"),
        "{normal:?}"
    );
    for t in tiers() {
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_wrong");
        assert_eq!(cycle, normal, "{t:?}");
        for (label, code) in [("list", "E4121"), ("latch", "E4119"), ("state", "E4120")] {
            let (_, r) = row(&err, &format!("cycle_wrong.{label}"));
            assert!(r.contains(&format!("  {code} ")), "{r}");
        }
    }
}

#[test]
fn cycle_strict_names_the_code_of_each_reason() {
    let dir = tempdir("cycle-strict-codes");
    analyze(&dir, "subset.vhd", include_str!("cycle/subset.vhd"));
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-strict",
            "sub_tb",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(o.stdout.is_empty(), "no simulation runs");
    for (code, label) in [
        ("E4110", "wait_on"),
        ("E4119", "latch"),
        ("E4122", "loop_x"),
        ("E4125", "gate"),
    ] {
        assert!(
            err.contains(&format!(
                "error[{code}]: process sub_tb.dut.{label} is outside what cycle mode can run"
            )),
            "{code} {label}: {err}"
        );
    }
    // In the subset but not implemented yet: the code of that.
    assert!(
        err.contains("error[E4101]: process sub_tb.dut.async_two cannot run cycle-based yet"),
        "{err}"
    );
    // The testbench, the top-level unit, is exempt, behavioural or not.
    assert!(!err.contains("process sub_tb.timed"), "{err}");
    assert!(!err.contains("process sub_tb.logger"), "{err}");
}

#[test]
fn ports_collapsed_into_parts_of_their_actuals_give_the_same_result() {
    let dir = tempdir("cycle-ports");
    analyze(&dir, "ports.vhd", include_str!("cycle/ports.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_ports");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_ports");
        assert_eq!(normal.len(), 6, "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 3 processes run cycle-based"),
            "{err}"
        );
    }
}

#[test]
fn asynchronous_resets_between_edges_give_the_same_result() {
    let dir = tempdir("cycle-reset");
    analyze(&dir, "reset.vhd", include_str!("cycle/reset.vhd"));
    for (generic, members) in [("-gCOMB=false", "4 of 5"), ("-gCOMB=true", "5 of 6")] {
        for t in tiers() {
            let flags = [&t[..], &[generic]].concat();
            let (normal, _) = run(&dir, &flags, "cycle_reset");
            let (cycle, err) = run(&dir, &[&flags[..], &["--cycle"]].concat(), "cycle_reset");
            // Hand-checked: the reset pulses before edges 5 and 13 and the level held at edge 9.
            assert_eq!(normal.len(), 20, "{normal:?}");
            assert!(normal[8].contains("n=09 m=40"), "{normal:?}");
            assert_eq!(cycle, normal, "{generic} {t:?}\n{err}");
            assert!(
                err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                "{err}"
            );
            // `ar2`, whose reset `rin` is a register of the region, joins.
            assert_eq!(row(&err, "cycle_reset.ar2").0, "cycle-based:", "{err}");
        }
    }
}

/// The rows of the processes named, each with the section it is in.
fn sections<'a>(report: &'a str, labels: &[&str]) -> Vec<(String, &'a str)> {
    labels
        .iter()
        .map(|l| (l.to_string(), row(report, l).0))
        .collect()
}

#[test]
fn several_clocks_and_crossings_give_the_same_result() {
    let dir = tempdir("cycle-clocks");
    analyze(&dir, "clocks.vhd", include_str!("cycle/clocks.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_clocks");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_clocks");
        assert_eq!(normal.len(), 24, "{normal:?}");
        // Hand-checked at 11.2 and 21.2 ns: at the coincident edge at 5 ns c2's registers
        // sampled c1's before the edge (x2 = 0), at 10 ns after it (x2 = 3); x1 = 0, with c2's y0
        // before the edge; cg's register on c1's edge at 15 ns, one delta later, saw c1's new
        // register (gx = 6); cu's first edge counts for `'event` only (nl = 2, nr = 1).
        assert!(
            normal[0].ends_with(
                "a1=03 a2=0A a3=02 af=04 x1=00 x2=03 x3=08 xf=00 x4=08 x5=02 w2=03 g1=00 \
                 gx=00 gs=00 nl=00 nr=00"
            ),
            "{normal:?}"
        );
        assert!(
            normal[1].ends_with("g1=01 gx=06 gs=07 nl=02 nr=01"),
            "{normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains(
                "cycle mode: 13 of 19 processes run cycle-based, on the rising edge of \
                 cycle_clocks.c2 and the rising edge of cycle_clocks.c1 and the falling edge of \
                 cycle_clocks.c1 and the rising edge of cycle_clocks.c3 and the rising edge of \
                 cycle_clocks.cg and the rising edge of cycle_clocks.cu (`'event` form) and the \
                 rising edge of cycle_clocks.cu"
            ),
            "{err}"
        );
        let names = [
            "cycle_clocks.p0",
            "cycle_clocks.p1",
            "cycle_clocks.p2",
            "cycle_clocks.pf",
            "cycle_clocks.s <= ...",
            "cycle_clocks.p3",
            "cycle_clocks.pw",
            "cycle_clocks.t <= ...",
            "cycle_clocks.p4",
            "cycle_clocks.pg",
            "cycle_clocks.pgs",
            "cycle_clocks.pl",
            "cycle_clocks.pr",
        ];
        for (name, section) in sections(&err, &names) {
            assert_eq!(section, "cycle-based:", "{name}\n{err}");
        }
        // The logic between c1 and the clock gated in the kernel, whose edges come a delta later,
        // stays in the kernel (with it in the region, `gs` differs); `t`, between c1 and c3 of two
        // clock processes waiting for known times, joins (their edges share a delta).
        let (section, line) = row(&err, "cycle_clocks.u <= ...");
        assert_eq!(
            section, "in the subset, not implemented in cycle mode yet:",
            "{err}"
        );
        assert!(line.contains("may come in a later delta"), "{line}");
    }
}

#[test]
fn asynchronous_resets_made_inside_the_region_give_the_same_result() {
    let dir = tempdir("cycle-inreset");
    analyze(&dir, "inreset.vhd", include_str!("cycle/inreset.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_inreset");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_inreset");
        assert_eq!(normal.len(), 30, "{normal:?}");
        // Hand-checked: held from time 0, released, n3 following din during the late pulse.
        assert!(normal[2].contains("r2='1' n1=10 n2=20 n3=36"), "{normal:?}");
        assert!(normal[3].contains("r2='0' n1=10 n2=30 n3=36"), "{normal:?}");
        assert!(
            normal[21].contains("r2='1' n1=10 n2=20 n3=37"),
            "{normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 11 of 14 processes run cycle-based"),
            "{err}"
        );
        let names = [
            "cycle_inreset.sync",
            "cycle_inreset.q1",
            "cycle_inreset.q2",
            "cycle_inreset.q3",
            "cycle_inreset.q4",
            "cycle_inreset.c2reg",
            "cycle_inreset.q5",
            "cycle_inreset.rc <= ...",
            "cycle_inreset.m4 <= ...",
            "cycle_inreset.p8",
            "cycle_inreset.q9",
        ];
        for (name, section) in sections(&err, &names) {
            assert_eq!(section, "cycle-based:", "{name}\n{err}");
        }
        // What a reset branch reads wakes the region, also another element of the reset's own
        // signal (n9 at 231.2 ns); the resets made inside do not.
        assert!(
            err.contains("2 wake the region (cycle_inreset.din, cycle_inreset.ctl)"),
            "{err}"
        );
        assert!(normal[22].ends_with("n9=FF"), "{normal:?}");
    }
}

#[test]
fn drivers_of_signals_whose_history_is_read_outside_stay_out() {
    let dir = tempdir("cycle-history");
    analyze(&dir, "history.vhd", include_str!("cycle/history.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_history");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_history");
        assert_eq!(normal.len(), 72, "{normal:?}");
        // With the drivers in the region the redrives of `r` vanish (`'transaction`, `'active`).
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 8 processes run cycle-based"),
            "{err}"
        );
        assert_eq!(row(&err, "cycle_history.dut.pn").0, "cycle-based:", "{err}");
        for (label, what) in [
            ("pq", "reads `'last_event` of cycle_history.q"),
            ("pr", "reads `'active` of cycle_history.r"),
            ("pt", "reads an implicit signal"),
            (
                "pk",
                "calls a subprogram that reads the history of cycle_history.k",
            ),
        ] {
            let (section, line) = row(&err, &format!("cycle_history.dut.{label}"));
            assert_eq!(section, "outside the subset, in the event kernel:", "{err}");
            assert!(
                line.contains("E4127 cycle_history.main") && line.contains(what),
                "{line}"
            );
        }
    }
    // Under `--cycle-strict` each is an error, though the reader is the testbench.
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-strict",
            "cycle_history",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    for label in ["pq", "pr", "pt", "pk"] {
        assert!(
            err.contains(&format!(
                "error[E4127]: process cycle_history.dut.{label} is outside what cycle mode can run"
            )),
            "{label}: {err}"
        );
    }
    assert!(err.contains("history.vhd:81:59"), "{err}");
}

/// The region reads only the inputs that changed since its last wake (`tests/cycle/inputs.vhd`):
/// inputs no wake is made for, changed between edges, changed back, changed in the delta of the
/// edge and at time 0, a wide input changed in one word, a record word holding an input field and
/// a member-driven one, an output read and driven back outside.
#[test]
fn inputs_read_only_when_they_changed_give_the_same_result() {
    let dir = tempdir("cycle-inputs");
    analyze(&dir, "inputs.vhd", include_str!("cycle/inputs.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_inputs");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_inputs");
        assert_eq!(normal.len(), 8, "{normal:?}");
        // Hand-checked: edge i samples what iteration i - 1 set. At edge 3 `s` held a5 (changed
        // back only in odd iterations), at edge 4 00; `w(101 to 103)` set, `n` 21, `e` toggled in
        // the edge's delta (4 times), `z` set at time 0, `m` its metavalues, `f` from `y(1)` of
        // the time before the edge, `y` from `c` (4) and `qs`, `wm` with `w(104)`, `r.a` toggled 3
        // times and `r.b` as it started; `p.b` (and `p2.a`) toggled when the input field was 0:
        // 1 1 0 0 1 1 0 0 over the edges. Reading `p.a` must not clear `p.b` in its word.
        assert!(normal[2].contains("i=3 qs=A5 "), "{normal:?}");
        assert!(
            normal[3].contains(
                "i=4 qs=00 qw=000000E000000000 qn=21 qe=0 qz=1 qf=1 qm=XX01 y=0100 wm=0000001E \
                 qr=11 pb=0 p2a=0"
            ),
            "{normal:?}"
        );
        // Two-state: `m` reads as 0 for its metavalues.
        let want: Vec<String> = normal
            .iter()
            .map(|l| {
                l.replace("qm=UUUU", "qm=0000")
                    .replace("qm=XX01", "qm=0001")
            })
            .collect();
        assert_eq!(cycle, want, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 3 of 5 processes run cycle-based"),
            "{err}"
        );
        // The 4 of `m` at initialisation, then its 2 once, at its change.
        assert!(
            err.contains(
                "cycle mode: 6 metavalue elements read as 0 (4 at initialisation, 2 at changes of \
                 the region's inputs)"
            ),
            "{err}"
        );
    }
}

const TABLE_INDEX: &str = "\
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_tbl is
end entity;
architecture a of cycle_tbl is
  type table_t is array (0 to 7) of unsigned(7 downto 0);
  constant TBL : table_t := (x\"11\", x\"22\", x\"33\", x\"44\", x\"55\", x\"66\", x\"77\", x\"88\");
  function rot(v : unsigned) return unsigned is
  begin
    return v(0) & v(v'left downto 1);
  end function;
  signal din : unsigned(7 downto 0) := x\"00\";
  signal k1, k2, k3, k4 : integer range 0 to 7 := 0;
  signal fo : unsigned(7 downto 0);
  signal fb : std_ulogic;
begin
  pf : process (all)
  begin
    k1 <= to_integer(din(2 downto 0));
    fo <= TBL(k2);
  end process;
  pg : process (all)
  begin
    k2 <= (k1 + 1) mod 8;
  end process;
  ph : process (all)
  begin
    k3 <= to_integer(din(2 downto 0));
    fb <= rot(din)(k4);
  end process;
  pk : process (all)
  begin
    k4 <= 7 - k3;
  end process;
  main : process
  begin
    for i in 1 to 6 loop
      din <= din + 3;
      wait for 1 ns;
      report \"fo=\" & to_hstring(fo) & \" fb=\" & std_ulogic'image(fb);
    end loop;
    wait;
  end process;
end architecture;
";

/// An element of a constant or of a function's result at an index chosen at run time depends
/// on the index: `pf` reads `k2` through `TBL(k2)`, which `pg` makes of what `pf` drives.
#[test]
fn an_element_chosen_at_run_time_depends_on_its_index() {
    let dir = tempdir("cycle-tbl");
    analyze(&dir, "tbl.vhd", TABLE_INDEX);
    let report = verdicts(&dir, "cycle_tbl");
    for group in [
        "loop group of 2 processes: 2 passes, or 3 pieces after a split: cycle_tbl.pf, cycle_tbl.pg",
        "loop group of 2 processes: 2 passes, or 3 pieces after a split: cycle_tbl.ph, cycle_tbl.pk",
    ] {
        assert!(report.contains(group), "{group}\n{report}");
    }
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_tbl");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_tbl");
        // Hand-checked: din 03, k1 3, k2 4; rot(din) is "10000001" indexed 0 to 7, k4 4.
        assert!(normal[0].ends_with("fo=55 fb='0'"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
    }
}

#[test]
fn loop_groups_run_in_the_region_and_give_the_same_result() {
    let dir = tempdir("cycle-loops");
    analyze(&dir, "loops.vhd", include_str!("cycle/loops.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_loops");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_loops");
        assert_eq!(normal.len(), 12, "{normal:?}");
        // Hand-checked at 5 ns (din 0D, din2 00, the reset held, the first edge one delta later):
        // every group settled, ob x3 from 1 ns before.
        assert!(
            normal[0].ends_with("o1=11 x3=1D t=57 q=14 w2=15 fo=33 rq=00 ob=1D"),
            "{normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 11 of 14 processes run cycle-based"),
            "{err}"
        );
        let names = [
            "cycle_loops.pa",
            "cycle_loops.pb",
            "cycle_loops.ca",
            "cycle_loops.cb",
            "cycle_loops.cc",
            "cycle_loops.sr",
            "cycle_loops.pw",
            "cycle_loops.pq",
            "cycle_loops.pf",
            "cycle_loops.pg",
            "cycle_loops.pr",
        ];
        for (name, section) in sections(&err, &names) {
            assert_eq!(section, "cycle-based:", "{name}\n{err}");
        }
        for group in [
            "loop group of 2 processes: 2 passes, or 3 pieces after a split: cycle_loops.pa, \
             cycle_loops.pb",
            "loop group of 3 processes: 3 passes, or 6 pieces after a split: cycle_loops.ca, \
             cycle_loops.cb, cycle_loops.cc",
            // `TBL(k2)`: an element of a constant depends on its index.
            "loop group of 2 processes: 2 passes, or 3 pieces after a split: cycle_loops.pf, \
             cycle_loops.pg",
            "loop group of 1 process: 2 passes, or 2 pieces after a split: cycle_loops.sr",
            "loop group of 2 processes: 2 passes, or 3 pieces after a split: cycle_loops.pw, \
             cycle_loops.pq",
        ] {
            assert!(err.contains(group), "{group}\n{err}");
        }
    }
}

#[test]
fn a_check_failing_in_a_loop_group_stops_the_run_as_in_the_normal_mode() {
    let dir = tempdir("cycle-loops-fail");
    analyze(&dir, "loops.vhd", include_str!("cycle/loops.vhd"));
    for mode in [None, Some("--cycle")] {
        let mut args = vec![
            "run",
            "--work=work:lib",
            "--no-cache",
            "--tier=interp",
            "-gFAIL=true",
        ];
        args.extend(mode);
        args.push("cycle_loops");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{mode:?}: {err}");
        assert!(
            err.contains("error[E3009]: value 8 outside of INTEGER range 0 to 7 for signal K2")
                && err.contains("in process cycle_loops.pg"),
            "{mode:?}: {err}"
        );
    }
}

const UNSETTLED_CHECK: &str = "\
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_early is
end entity;
architecture a of cycle_early is
  type table_t is array (0 to 6) of unsigned(7 downto 0);
  constant TBL : table_t := (x\"10\", x\"11\", x\"12\", x\"13\", x\"14\", x\"15\", x\"16\");
  signal din, h, x, y, r2 : unsigned(7 downto 0) := x\"03\";
  signal r1 : unsigned(7 downto 0);
begin
  h <= din;
  x <= h + h;
  r : process (all)
  begin
    r2 <= din;
    r1 <= TBL(to_integer(x - y));
  end process;
  y <= r2;
  main : process
  begin
    wait for 1 ns;
    report \"r1=\" & to_hstring(r1);
    din <= x\"05\";
    wait for 1 ns;
    report \"r1=\" & to_hstring(r1);
    wait;
  end process;
end architecture;
";

/// `r` and `y` form a loop group; after din goes from 3 to 5, `r`'s first run sees x = 10 and the
/// old y = 3 (index 7, out of range), which the normal mode never evaluates: only the run on
/// settled values is checked.
#[test]
fn a_check_on_values_not_settled_yet_does_not_stop_the_run() {
    let dir = tempdir("cycle-early");
    analyze(&dir, "early.vhd", UNSETTLED_CHECK);
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_early");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_early");
        assert_eq!(normal.len(), 2, "{normal:?}");
        assert!(normal[1].ends_with("r1=15"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 4 of 5 processes run cycle-based"),
            "{err}"
        );
        assert_eq!(row(&err, "cycle_early.r").0, "cycle-based:", "{err}");
    }
}

/// A run of `top` with `env` set, as (standard output, standard error, exit status).
fn run_env(dir: &Path, env: &[(&str, &str)], flags: &[&str], top: &str) -> (String, String, i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .envs(env.iter().copied())
        .args(["--color", "never", "run", "--work=work:lib", "--no-cache"])
        .args(flags)
        .arg(top)
        .output()
        .expect("run yavs");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.code().unwrap_or(-1),
    )
}

/// Members cut into several compiled functions: a combinational variable live across every part,
/// an `if` cut inside its branches, branches in the unrolled loops, and a range check in the last
/// part of a clocked member that fails as in the normal mode. At the size the code generator
/// cuts by itself and with `YAVS_CYCLE_CHUNK=7`, which cuts every member into parts of a few
/// operations.
#[test]
fn members_cut_into_several_functions_give_the_same_result() {
    let dir = tempdir("cycle-cut");
    analyze(&dir, "cut.vhd", include_str!("cycle/cut.vhd"));
    let lines = |out: &str| -> Vec<String> {
        (out.lines())
            .filter(|l| l.contains("c="))
            .map(String::from)
            .collect()
    };
    let (normal, err, rc) = run_env(&dir, &[], &["--tier=interp"], "cycle_cut");
    assert_eq!(rc, 0, "{err}");
    let normal = lines(&normal);
    assert_eq!(normal.len(), 12, "{normal:?}");
    // The normal mode's error of the failing check: message and where.
    let (_, nerr, nrc) = run_env(&dir, &[], &["--tier=interp", "-gLIMIT=5000"], "cycle_cut");
    assert_eq!(nrc, 1, "{nerr}");
    let failure: Vec<&str> = (nerr.lines())
        .filter(|l| l.starts_with("error[E3009]") || l.contains("= note: at "))
        .map(str::trim)
        .collect();
    assert_eq!(failure.len(), 2, "{nerr}");
    // The last also with two words in registers at a time (`YAVS_CYCLE_REGS`).
    for (chunk, cap) in [(None, None), (Some("7"), None), (Some("7"), Some("2"))] {
        let mut env = vec![("YAVS_CYCLE_STATS", "1")];
        env.extend(chunk.map(|c| ("YAVS_CYCLE_CHUNK", c)));
        env.extend(cap.map(|c| ("YAVS_CYCLE_REGS", c)));
        let (out, err, rc) = run_env(&dir, &env, &["--tier=interp", "--cycle"], "cycle_cut");
        assert_eq!(rc, 0, "{chunk:?}: {err}");
        assert_eq!(lines(&out), normal, "{chunk:?}");
        assert!(
            err.contains("cycle mode: 3 of 4 processes run cycle-based"),
            "{err}"
        );
        // Both large members in several functions.
        let funs = (err.lines())
            .filter(|l| l.starts_with("cycle stats: cycle_"))
            .count();
        assert!(funs >= 6, "{chunk:?}: {funs} functions: {err}");
        let (_, err, rc) = run_env(
            &dir,
            &env,
            &["--tier=interp", "--cycle", "-gLIMIT=5000"],
            "cycle_cut",
        );
        assert_eq!(rc, 1, "{chunk:?}: {err}");
        for l in &failure {
            assert!(err.contains(l), "{chunk:?}: {l}: {err}");
        }
    }
    // The region from the cache decodes its check sites when the check fails.
    for pass in ["cold", "warm"] {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&dir)
            .env("YAVS_STD_CACHE", "off")
            .env("YAVS_CYCLE_STATS", "1")
            .args([
                "--color",
                "never",
                "run",
                "--work=work:lib",
                "--tier=interp",
            ])
            .args(["--cycle", "-gLIMIT=5000", "cycle_cut"])
            .output()
            .expect("run yavs");
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{pass}: {err}");
        assert_eq!(
            err.contains("region from the cache"),
            pass == "warm",
            "{err}"
        );
        for l in &failure {
            assert!(err.contains(l), "{pass}: {l}: {err}");
        }
    }
}

/// Instances sharing one compiled body (`share.vhd`): two of one generic share one body, two of
/// another generic (a constant folded into the code) another, each driving its own signals from
/// its own input; a failed check in a shared body names its own instance. Also with every member
/// cut into parts (`YAVS_CYCLE_CHUNK=7`).
#[test]
fn instances_of_one_shape_share_a_body_and_others_do_not() {
    let dir = tempdir("cycle-share");
    analyze(&dir, "share.vhd", include_str!("cycle/share.vhd"));
    let lines = |out: &str| -> Vec<String> {
        (out.lines())
            .filter(|l| l.contains("c="))
            .map(String::from)
            .collect()
    };
    let (normal, err, rc) = run_env(&dir, &[], &["--tier=interp"], "cycle_share");
    assert_eq!(rc, 0, "{err}");
    let normal = lines(&normal);
    assert_eq!(normal.len(), 10, "{normal:?}");
    let (_, nerr, nrc) = run_env(&dir, &[], &["--tier=interp", "-gFAIL=true"], "cycle_share");
    assert_eq!(nrc, 1, "{nerr}");
    let failure: Vec<&str> = (nerr.lines())
        .filter(|l| l.starts_with("error[E3009]") || l.contains("= note: at "))
        .map(str::trim)
        .collect();
    assert_eq!(failure.len(), 2, "{nerr}");
    assert!(failure[1].ends_with("cycle_share.u1.p"), "{nerr}");
    // The last also with two words in registers at a time (`YAVS_CYCLE_REGS`).
    for (chunk, cap) in [(None, None), (Some("7"), None), (Some("7"), Some("2"))] {
        let mut env = vec![("YAVS_CYCLE_STATS", "1")];
        env.extend(chunk.map(|c| ("YAVS_CYCLE_CHUNK", c)));
        env.extend(cap.map(|c| ("YAVS_CYCLE_REGS", c)));
        let (out, err, rc) = run_env(&dir, &env, &["--tier=interp", "--cycle"], "cycle_share");
        assert_eq!(rc, 0, "{chunk:?}: {err}");
        assert_eq!(lines(&out), normal, "{chunk:?}");
        assert!(
            err.contains("cycle mode: 8 of 9 processes run cycle-based"),
            "{err}"
        );
        assert!(
            err.contains("cycle stats: 4 members share 2 bodies"),
            "{chunk:?}: {err}"
        );
        let (_, err, rc) = run_env(
            &dir,
            &env,
            &["--tier=interp", "--cycle", "-gFAIL=true"],
            "cycle_share",
        );
        assert_eq!(rc, 1, "{chunk:?}: {err}");
        for l in &failure {
            assert!(err.contains(l), "{chunk:?}: {l}: {err}");
        }
    }
}

/// Asserts and reports of the top-level unit (A19): the clocked ones evaluated at the edge in
/// region code (a checker failing with severity error, a passing one, one checking in a called
/// procedure, one right after a `wait until`, one formatting vectors of `W` elements, logic values, a boolean, integers and
/// enumerations, a variable changed after its report), the concurrent assert and the
/// combinational report in the event kernel; every message, its time and process, and the exit
/// status are the normal mode's, also when a checker of severity failure ends the run. Not an
/// error under `--cycle-strict`.
#[test]
fn asserts_and_reports_of_the_top_level_unit_are_evaluated() {
    let dir = tempdir("cycle-topcheck");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("topcheck{w}.vhd"),
            &with_width(include_str!("cycle/topcheck.vhd"), w),
        );
        let go = |flags: &[&str], top: &str| {
            let mut args = vec!["run", "--work=work:lib", "--no-cache"];
            args.extend(flags);
            args.push(top);
            let o = yavs(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            // Without the delta of each stamp: the region saves the deltas of its logic.
            let out: String = String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| match l.split_once("fs+") {
                    Some((a, b)) => format!(
                        "{a}fs{}\n",
                        b.trim_start_matches(|c: char| c.is_ascii_digit())
                    ),
                    None => format!("{l}\n"),
                })
                .collect();
            (o.status.code(), out, err)
        };
        let (code, normal, _) = go(&[], "cycle_topcheck");
        for msg in [
            "q reached 3",
            "q 12 not below 12",
            "q is 9",
            "q is 6",
            "q is 5 at the edge",
            "fmt ",
            "n=4 ",
            "Assertion violation.",
            "flag unknown",
        ] {
            assert!(normal.contains(msg), "{w} {msg}: {normal}");
        }
        assert!(!normal.contains("never"), "{normal}");
        assert_eq!(code, Some(1), "{normal}");
        let (stop_code, stop, _) = go(&[], "cycle_topstop");
        assert!(stop.contains("q reached 5"), "{stop}");
        for mode in ["--cycle", "--cycle-strict"] {
            for t in tiers() {
                let flags = [&t[..], &[mode]].concat();
                let (c, out, err) = go(&flags, "cycle_topcheck");
                assert_eq!((c, &out), (code, &normal), "{w} {flags:?}\n{err}");
                // The design below the top and the clocked checkers.
                assert!(
                    err.contains("cycle mode: 8 of 13 processes run cycle-based"),
                    "{w} {mode}: {err}"
                );
                let (c, out, err) = go(&flags, "cycle_topstop");
                assert_eq!((c, &out), (stop_code, &stop), "{w} {flags:?}\n{err}");
            }
        }
        let report = with_report(&dir, &["--cycle"], "cycle_topcheck", String::new());
        assert_eq!(
            report
                .matches("it holds an assert or report of the top-level unit that cycle mode evaluates only in the event kernel: an assert or report that runs outside a clock edge")
                .count(),
            2,
            "{report}"
        );
        for p in ["rep", "assert"] {
            assert!(
                report.contains(&format!("cycle_topcheck.{p} ")),
                "{p}: {report}"
            );
        }
        assert!(
            report
                .contains("it checks cycle_topcheck.flag, which is not driven by the region alone"),
            "{report}"
        );
        assert!(
            !report.contains("not evaluated in cycle mode, held by"),
            "{report}"
        );
    }
}

/// A cycle run with the compilation cache of the work library on and `YAVS_CYCLE_STATS`: its
/// report line, the functions compiled and how many of them came from the cache.
fn cached(dir: &Path, env: &[(&str, &str)], flags: &[&str]) -> (String, usize, usize) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .env("YAVS_CYCLE_STATS", "1")
        .envs(env.iter().copied())
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--tier=interp",
        ])
        .args(flags)
        .arg("cycle_cache")
        .output()
        .expect("run yavs");
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    assert!(o.status.success(), "{env:?} {flags:?}: {out}{err}");
    let report: Vec<&str> = out.lines().filter(|l| l.contains("cnt=")).collect();
    assert_eq!(report.len(), 1, "{out}");
    // "cycle stats: N functions in T ms, H from the cache"
    let total = err
        .lines()
        .find_map(|l| {
            l.strip_prefix("cycle stats: ")?
                .split_once(" functions in ")
        })
        .unwrap_or_else(|| panic!("no total line: {err}"));
    let hits = total.1.split(", ").nth(1).and_then(|h| h.split(' ').next());
    (
        report[0].to_string(),
        total.0.parse().expect("functions"),
        hits.and_then(|h| h.parse().ok()).expect("hits"),
    )
}

/// Functions compiled on several threads are those of one thread: a run on four threads gives the
/// result of one, and the cache check of a run on one thread finds every entry it left the same;
/// `--threads` sets the count, and a warm run takes every function.
#[test]
fn functions_compiled_on_several_threads_are_those_of_one() {
    let dir = tempdir("cycle-par");
    analyze(&dir, "cache.vhd", include_str!("cycle/cache.vhd"));
    let obj = dir.join("lib").join("yavs-cache");
    // Members cut into parts of a few operations: many functions.
    let one = [("YAVS_CYCLE_CHUNK", "5"), ("YAVS_CYCLE_THREADS", "1")];
    let (want, n, _) = cached(&dir, &one, &["--cycle", "--no-cache"]);
    assert!(n >= 4, "{n} functions");
    let four = [("YAVS_CYCLE_CHUNK", "5"), ("YAVS_CYCLE_THREADS", "4")];
    let (got, n4, hits) = cached(&dir, &four, &["--cycle"]);
    assert_eq!((&got, n4, hits), (&want, n, 0), "four threads, cold");
    let check = [
        ("YAVS_CYCLE_CHUNK", "5"),
        ("YAVS_CYCLE_THREADS", "1"),
        ("YAVS_CYCLE_CACHE_CHECK", "1"),
    ];
    let (got, _, hits) = cached(&dir, &check, &["--cycle"]);
    assert_eq!(
        (&got, hits),
        (&want, 0),
        "one thread, every function checked"
    );
    std::fs::remove_dir_all(&obj).expect("empty the cache");
    let chunk = [("YAVS_CYCLE_CHUNK", "5")];
    let (got, _, hits) = cached(&dir, &chunk, &["--cycle", "--threads=4"]);
    assert_eq!((&got, hits), (&want, 0), "--threads=4, cold");
    let (got, _, hits) = cached(&dir, &chunk, &["--cycle", "--threads=4"]);
    assert_eq!((&got, hits), (&want, n), "--threads=4, warm");
}

/// The machine code of the region in the work library's compilation cache: a second run takes
/// every function from it and prints the same; another generic, another source line, other
/// checks and another build of yavs each compile again what changed; damaged entries are
/// misses; `--no-cache` neither reads nor writes it; two runs racing on an empty cache both
/// succeed.
#[test]
fn region_code_from_the_cache_gives_the_same_result() {
    let dir = tempdir("cycle-cache");
    analyze(&dir, "cache.vhd", include_str!("cycle/cache.vhd"));
    let normal = |flags: &[&str]| {
        let (out, _, rc) = run_env(&dir, &[], flags, "cycle_cache");
        assert_eq!(rc, 0, "{out}");
        out.lines()
            .find(|l| l.contains("cnt="))
            .expect("report")
            .to_string()
    };
    let want = normal(&["--tier=interp"]);
    assert!(want.ends_with("cnt=44 acc=738"), "{want}");
    let obj = dir.join("lib").join("yavs-cache").join("cycle");
    let entries = || -> Vec<std::path::PathBuf> {
        (std::fs::read_dir(&obj).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("cycle-") || n.starts_with("cyc3")
                })
            })
            .collect()
    };

    let no_cache = cached(&dir, &[], &["--cycle", "--no-cache"]);
    assert_eq!((no_cache.0.as_str(), no_cache.2), (want.as_str(), 0));
    assert!(entries().is_empty(), "--no-cache wrote {:?}", entries());

    let (cold, n, hits) = cached(&dir, &[], &["--cycle"]);
    assert_eq!((cold.as_str(), hits), (want.as_str(), 0));
    assert!(n > 0 && !entries().is_empty());
    let (warm, n2, hits) = cached(&dir, &[], &["--cycle"]);
    assert_eq!(
        (warm.as_str(), n2, hits),
        (want.as_str(), n, n),
        "a warm run"
    );

    // Each change that makes other code misses where it does, and gives the normal mode's result.
    let w12 = normal(&["--tier=interp", "-gW=12"]);
    let checks = [("YAVS_CYCLE_CHECKS", "off")];
    let salt = [("YAVS_CACHE_SALT", "another build")];
    for (what, env, flags, want) in [
        ("generic", &[][..], &["--cycle", "-gW=12"][..], &w12),
        ("checks", &checks[..], &["--cycle"][..], &want),
        ("build", &salt[..], &["--cycle"][..], &want),
    ] {
        let (got, n, hits) = cached(&dir, env, flags);
        assert_eq!(&got, want, "{what}");
        assert!(hits < n, "{what}: {hits} of {n} functions from the cache");
        if what == "build" {
            assert_eq!(hits, 0, "another build took code from the cache");
        }
        let (got, n2, hits) = cached(&dir, env, flags);
        assert_eq!((&got, n2, hits), (want, n, n), "{what}, warm");
    }
    // The register cap is in the key: the check compiles every function again and fails on
    // code made for another cap.
    let regs = [("YAVS_CYCLE_REGS", "2"), ("YAVS_CYCLE_CACHE_CHECK", "1")];
    let (got, _, hits) = cached(&dir, &regs, &["--cycle"]);
    assert_eq!((&got, hits), (&want, 0), "two words in registers");
    analyze(
        &dir,
        "cache.vhd",
        &include_str!("cycle/cache.vhd").replace("STEP : natural := 3", "STEP : natural := 5"),
    );
    let step5 = normal(&["--tier=interp"]);
    assert_ne!(step5, want);
    let (got, n, hits) = cached(&dir, &[], &["--cycle"]);
    assert_eq!(got, step5, "an edited source line");
    assert!(
        hits < n,
        "an edited source line: {hits} of {n} from the cache"
    );

    // Damaged entries: cut short, then garbage. Each is a miss and compiled again.
    for damage in [0, 1] {
        for p in entries() {
            let bytes = std::fs::read(&p).expect("entry");
            let bad = if damage == 0 {
                bytes[..bytes.len() / 2].to_vec()
            } else {
                vec![0x5a; bytes.len()]
            };
            std::fs::write(&p, bad).expect("damage");
        }
        let (got, n, hits) = cached(&dir, &[], &["--cycle"]);
        assert_eq!((&got, hits), (&step5, 0), "damage {damage}");
        let (got, n2, hits) = cached(&dir, &[], &["--cycle"]);
        assert_eq!((&got, n2, hits), (&step5, n, n), "damage {damage}, warm");
    }

    // Two runs at once on an empty cache.
    for p in entries() {
        std::fs::remove_file(p).expect("remove entry");
    }
    let racers: Vec<_> = (0..2)
        .map(|_| {
            let d = dir.to_path_buf();
            std::thread::spawn(move || cached(&d, &[], &["--cycle", "-gW=9"]))
        })
        .collect();
    let w9 = normal(&["--tier=interp", "-gW=9"]);
    for r in racers {
        assert_eq!(r.join().expect("racer").0, w9);
    }
    let (got, n, hits) = cached(&dir, &[], &["--cycle", "-gW=9"]);
    assert_eq!((&got, hits), (&w9, n));
}

/// A cycle run of `acache` with the caches of the work library on and `YAVS_CYCLE_STATS`: the
/// testbench's line, what the analysis cache did (its stats line) and the stats line of the
/// region's functions.
fn acached(dir: &Path, env: &[(&str, &str)], flags: &[&str]) -> (String, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .env("YAVS_CYCLE_STATS", "1")
        .envs(env.iter().copied())
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--tier=interp",
        ])
        .args(flags)
        .arg("acache")
        .output()
        .expect("run yavs");
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    assert!(o.status.success(), "{env:?} {flags:?}: {out}{err}");
    let line = |text: &str, what: &str| {
        (text.lines())
            .find(|l| l.contains(what))
            .map_or(String::new(), |l| l.to_string())
    };
    let cache: Vec<&str> = (err.lines())
        .filter(|l| l.contains("cycle stats: analysis") || l.contains("cycle stats: region"))
        .collect();
    (
        line(&out, "cnt="),
        cache.join("\n"),
        line(&err, " functions in "),
    )
}

/// The analysis cache: a second run takes the selection (and the region's code) from the cache
/// and prints the same; a generic nothing reads before the run (`TAG`) hits, one the walk, the
/// value passes or the selection reads misses, and so do an edited dependency, another
/// environment and another build; damaged entries are misses; `--no-cache` writes nothing; two
/// runs racing on an empty cache; `YAVS_CYCLE_CACHE_CHECK=1` makes it again and compares.
#[test]
fn the_analysis_from_the_cache_gives_the_same_result() {
    let dir = tempdir("cycle-acache");
    analyze(&dir, "acache.vhd", include_str!("cycle/acache.vhd"));
    let normal = |flags: &[&str]| {
        let (out, _, rc) = run_env(&dir, &[], flags, "acache");
        assert_eq!(rc, 0, "{out}");
        out.lines()
            .find(|l| l.contains("cnt="))
            .expect("report")
            .to_string()
    };
    let store = dir.join("lib").join("yavs-cache").join("cycle");
    let entries = || -> Vec<std::path::PathBuf> {
        (std::fs::read_dir(&store).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("cyc3"))
            })
            .collect()
    };
    let (stored, hit) = ("analysis stored", "analysis from the cache");
    let want = normal(&["--tier=interp"]);
    assert!(want.ends_with("x cnt=60 a=60 b=80"), "{want}");

    let (got, a, _) = acached(&dir, &[], &["--cycle", "--no-cache"]);
    assert_eq!((got.as_str(), a.as_str()), (want.as_str(), ""));
    assert!(entries().is_empty(), "--no-cache wrote {:?}", entries());

    let (got, a, _) = acached(&dir, &[], &["--cycle"]);
    assert_eq!(got, want);
    assert!(
        a.contains(stored)
            && a.contains("generics W (walk), L (walk), F (value passes), K (selection)"),
        "{a}"
    );
    assert!(!a.contains("TAG"), "{a}");
    let (got, a, f) = acached(&dir, &[], &["--cycle"]);
    assert_eq!(got, want);
    assert!(a.contains(hit), "a warm run: {a}");
    assert!(a.contains("region from the cache"), "a warm run: {a}");
    assert!(f.contains("from the cache"), "{f}");
    let check = [("YAVS_CYCLE_CACHE_CHECK", "1")];
    let (got, a, _) = acached(&dir, &check, &["--cycle"]);
    assert_eq!(got, want);
    assert!(
        a.contains("analysis from the cache checked: the same"),
        "{a}"
    );
    assert!(a.contains("region from the cache checked: the same"), "{a}");
    // `--cycle-strict` builds the region from the selection in the cache, with its code.
    let (got, a, f) = acached(&dir, &[], &["--cycle-strict"]);
    assert_eq!(got, want);
    assert!(a.contains(hit) && !a.contains("region from"), "strict: {a}");
    assert!(f.contains("from the cache"), "strict: {f}");
    // The built region damaged alone: the selection in the cache builds it again.
    for p in entries()
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("cyc3c-"))
    {
        std::fs::write(&p, b"damaged").expect("damage");
    }
    let (got, a, _) = acached(&dir, &[], &["--cycle"]);
    assert_eq!(got, want);
    assert!(
        a.contains(hit) && !a.contains("region from"),
        "damaged region: {a}"
    );
    let (got, a, _) = acached(&dir, &[], &["--cycle"]);
    assert_eq!(got, want);
    assert!(a.contains("region from the cache"), "rebuilt region: {a}");

    // Read by nothing before the run: the same analysis.
    let (got, a, _) = acached(&dir, &[], &["--cycle", "-gTAG=y"]);
    assert_eq!(got, normal(&["--tier=interp", "-gTAG=y"]));
    assert!(a.contains(hit), "TAG: {a}");

    // Read by the walk (W, L), the value passes or the selection (F, K), another environment, another
    // build: made again, right, then from the cache.
    let excl = [("YAVS_CYCLE_EXCLUDE", "nothing_has_this_name")];
    let salt = [("YAVS_CACHE_SALT", "another build")];
    // Another key whose analysis and region come out the same keeps no second copy of either.
    let payloads = || {
        (entries().iter())
            .filter(|p| p.to_string_lossy().contains("cyc3p-"))
            .count()
    };
    let (n, all) = (payloads(), entries().len());
    let other = [("YAVS_CYCLE_EXCLUDE", "nor_has_this_one")];
    let (got, a, _) = acached(&dir, &other, &["--cycle"]);
    assert_eq!(got, want);
    assert!(a.contains(stored), "another environment: {a}");
    assert_eq!((n, payloads()), (2, 2), "the same payloads stored again");
    assert!(entries().len() > all, "no entries of the new key");
    let (got, a, _) = acached(&dir, &other, &["--cycle"]);
    assert_eq!(got, want);
    assert!(
        a.contains("region from the cache"),
        "another environment, warm: {a}"
    );
    for (what, env, flags) in [
        ("W", &[][..], &["-gW=9"][..]),
        ("L", &[][..], &["-gL=abc"][..]),
        ("F", &[][..], &["-gF=5"][..]),
        ("K", &[][..], &["-gK=7"][..]),
        ("environment", &excl[..], &[][..]),
        ("build", &salt[..], &[][..]),
    ] {
        let want = normal(&[&["--tier=interp"][..], flags].concat());
        let flags = [&["--cycle"][..], flags].concat();
        let (got, a, _) = acached(&dir, env, &flags);
        assert_eq!(got, want, "{what}");
        assert!(a.contains(stored), "{what}: {a}");
        let (got, a, _) = acached(&dir, env, &flags);
        assert_eq!(got, want, "{what}, warm");
        assert!(a.contains(hit), "{what}, warm: {a}");
    }

    // At most 16 analyses per design: `K` read by the selection makes another one each time.
    let mut last = String::new();
    for k in 10..=30 {
        let flags = ["--cycle".to_string(), format!("-gK={k}")];
        let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
        last = acached(&dir, &[], &flags).1;
    }
    assert!(
        last.contains("16 analyses of this design are kept"),
        "{last}"
    );

    // An edited dependency.
    analyze(
        &dir,
        "acache.vhd",
        &include_str!("cycle/acache.vhd").replace("c <= c + 1 + N;", "c <= c + 2 + N;"),
    );
    let edited = normal(&["--tier=interp"]);
    assert_ne!(edited, want);
    let (got, a, _) = acached(&dir, &[], &["--cycle"]);
    assert_eq!(got, edited, "an edited dependency");
    assert!(a.contains(stored), "an edited dependency: {a}");

    // Damaged entries: cut short, then garbage. Each is a miss and made again.
    for damage in [0, 1] {
        for p in entries() {
            let bytes = std::fs::read(&p).expect("entry");
            let bad = if damage == 0 {
                bytes[..bytes.len() / 2].to_vec()
            } else {
                vec![0x5a; bytes.len()]
            };
            std::fs::write(&p, bad).expect("damage");
        }
        let (got, a, _) = acached(&dir, &[], &["--cycle"]);
        assert_eq!(got, edited, "damage {damage}");
        assert!(!a.contains(hit), "damage {damage}: {a}");
    }

    // Two runs at once on an empty cache, as a test runner's parallel jobs do.
    for p in entries() {
        std::fs::remove_file(p).expect("remove entry");
    }
    let w9 = normal(&["--tier=interp", "-gW=9"]);
    let racers: Vec<_> = (0..2)
        .map(|_| {
            let d = dir.to_path_buf();
            std::thread::spawn(move || acached(&d, &[], &["--cycle", "-gW=9"]).0)
        })
        .collect();
    for r in racers {
        assert_eq!(r.join().expect("racer"), w9);
    }
    let (got, a, _) = acached(&dir, &[], &["--cycle", "-gW=9"]);
    assert_eq!(got, w9);
    assert!(a.contains(hit), "after the race: {a}");
}

/// An elaboration that reads a file (a constant from an impure function of `textio`) keeps the
/// file's bytes in the key of its analysis: a run after the file changed follows it, and a run on
/// the same bytes takes the analysis from the cache. One that also writes a file keeps none.
#[test]
fn an_elaboration_that_reads_a_file_keys_its_bytes() {
    let dir = tempdir("cycle-acfile");
    analyze(&dir, "acfile.vhd", include_str!("cycle/acfile.vhd"));
    let run_env = |step: &str, flags: &[&str], env: &[(&str, &str)], cache: &str| {
        std::fs::write(dir.join("acfile.txt"), format!("{step}\n")).expect("write");
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&dir)
            .env("YAVS_STD_CACHE", "off")
            .env("YAVS_CYCLE_STATS", "1")
            .envs(env.iter().copied())
            .args([
                "--color",
                "never",
                "run",
                "--work=work:lib",
                "--tier=interp",
            ])
            .args(["--cycle", "acfile"])
            .args(flags)
            .output()
            .expect("run yavs");
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        );
        assert!(o.status.success(), "{out}{err}");
        assert!(err.contains("run cycle-based"), "{err}");
        assert!(err.contains(cache), "{step} {flags:?}: {err}");
        out.lines()
            .find(|l| l.contains("a="))
            .expect("report")
            .to_string()
    };
    let run = |step: &str, flags: &[&str], cache: &str| run_env(step, flags, &[], cache);
    let (stored, hit) = ("analysis stored", "analysis from the cache");
    assert!(run("3", &[], stored).ends_with("a=30"));
    assert!(run("3", &[], hit).ends_with("a=30"));
    assert!(run("5", &[], stored).ends_with("a=50"));
    assert!(run("5", &[], hit).ends_with("a=50"));
    assert!(run("3", &[], hit).ends_with("a=30"));
    let check = [("YAVS_CYCLE_CACHE_CHECK", "1")];
    assert!(run_env("5", &[], &check, "checked: the same").ends_with("a=50"));
    let wrote = "analysis not cached: elaboration wrote a file";
    assert!(run("4", &["-gWR=true"], wrote).ends_with("a=40"));
    assert!(run("4", &["-gWR=true"], wrote).ends_with("a=40"));
}

/// A region of more functions than the store keeps is not cached: it could never be warm, and
/// its writes would fill the store with entries of one design (and, before the store looked at a
/// full directory once and not at every put, a run took minutes). Every process is cut into
/// functions of one operation, so a few thousand processes are more than the 16384 entries.
#[test]
fn a_region_of_more_functions_than_the_cache_holds_is_not_cached() {
    let dir = tempdir("cycle-many");
    analyze(&dir, "many.vhd", include_str!("cycle/many.vhd"));
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&dir)
        .env("YAVS_STD_CACHE", "off")
        .env("YAVS_CYCLE_STATS", "1")
        .env("YAVS_CYCLE_CHUNK", "1")
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--tier=interp",
        ])
        .args(["--cycle", "cycle_many"])
        .output()
        .expect("run yavs");
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    assert!(o.status.success(), "{out}{err}");
    assert!(out.contains("link(3)="), "{out}");
    let n: usize = (err.lines())
        .find_map(|l| {
            l.strip_prefix("cycle stats: ")?
                .split_once(" functions in ")
        })
        .map(|t| t.0.parse().expect("functions"))
        .unwrap_or_else(|| panic!("no total line: {err}"));
    assert!(n > 16384, "{n} functions");
    let obj = dir.join("lib").join("yavs-cache").join("cycle");
    // The function entries; the analysis and the region's code as a whole are one entry each.
    let cached = (std::fs::read_dir(&obj).into_iter().flatten().flatten())
        .filter(|e| e.file_name().to_string_lossy().starts_with("cycle-"))
        .count();
    assert_eq!(cached, 0, "entries of a region that cannot fit");
}

const SUBSET_CODES: &str = "\
library ieee;
use ieee.std_logic_1164.all;
entity cycle_codes is
end entity;
architecture a of cycle_codes is
  signal clk, en, d, q2, q3, q4, q5, q7, q9, q10 : std_logic := '0';
  procedure waits(signal s : in std_logic) is
  begin
    wait until s = '1';
  end procedure;
begin
  edge_in_clocked : process (clk)
  begin
    if rising_edge(clk) then
      if rising_edge(en) then q2 <= d; end if;
    end if;
  end process;
  edge_in_comb : process (all)
  begin
    if rising_edge(en) then q3 <= d; else q3 <= '0'; end if;
  end process;
  event_in_clocked : process (clk, en)
  begin
    if rising_edge(clk) then
      if en'event then q4 <= d; end if;
    end if;
  end process;
  event_in_comb : process (all)
  begin
    if en'event then q5 <= d; else q5 <= '0'; end if;
  end process;
  calls_wait : process
  begin
    waits(en);
    q7 <= d;
  end process;
  driving : process (all)
  begin
    q9 <= en'driving_value;
  end process;
  edge_after_elsif : process (clk, en)
  begin
    if rising_edge(clk) then q10 <= d; elsif en'event and en'last_value = '0' then q10 <= '0'; end if;
  end process;
end architecture;
";

/// What the texts of E4110, E4112 and E4123 say matches the codes the checker gives: an edge test
/// of another signal is E4112 inside a clocked body and E4123 where the shape is not clocked or
/// has more after the edge; a procedure that waits makes a process behavioural, with no code.
#[test]
fn explain_texts_of_the_subset_codes_match_what_the_checker_gives() {
    let dir = tempdir("cycle-codes");
    analyze(&dir, "codes.vhd", SUBSET_CODES);
    let o = yavs(
        &dir,
        &["run", "--work=work:lib", "--cycle-report", "cycle_codes"],
    );
    let report = String::from_utf8_lossy(&o.stderr);
    let line = |p: &str| {
        let row = report
            .lines()
            .find(|l| l.contains(&format!("cycle_codes.{p} ")));
        row.unwrap_or_else(|| panic!("{p}: {report}")).to_string()
    };
    for (p, code) in [
        ("edge_in_clocked", "E4112"),
        ("event_in_clocked", "E4112"),
        ("event_in_comb", "E4112"),
        ("driving", "E4112"),
        ("edge_in_comb", "E4123"),
        ("edge_after_elsif", "E4123"),
    ] {
        assert!(line(p).contains(&format!("{code} ")), "{p}: {report}");
    }
    let wait = report
        .find("behavioural, in the event kernel")
        .expect("a group");
    assert!(
        report[wait..].contains("cycle_codes.calls_wait "),
        "{report}"
    );
    assert!(!line("calls_wait").contains("E41"), "{report}");
    let explain = |code: &str| {
        let o = yavs(&dir, &["--explain", code]);
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let e4112 = explain("E4112");
    assert!(
        e4112.contains("`'driving_value`")
            && e4112.contains("inside the body of a clocked process")
            && e4112.contains("is E4123"),
        "{e4112}"
    );
    let e4110 = explain("E4110");
    assert!(
        !e4110.contains("a call of a procedure that waits")
            && e4110.contains("calls a procedure that waits is behavioural"),
        "{e4110}"
    );
}

/// The range of a subtype of an architecture that elaboration evaluates per instance (a function
/// that folding does not run) as a slice range and a loop range, in instances of 1, W and W + 4
/// elements, against the normal mode.
#[test]
fn ranges_elaboration_evaluated_per_instance_equal_the_normal_mode() {
    let dir = tempdir("cycle-subrange");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("subrange{w}.vhd"),
            &with_width(include_str!("cycle/subrange.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_subrange");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_subrange");
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main`.
            assert!(
                err.contains("cycle mode: 6 of 7 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

#[test]
fn members_skipped_on_unchanged_inputs_give_the_same_result() {
    let dir = tempdir("cycle-skip");
    analyze(&dir, "skip.vhd", include_str!("cycle/skip.vhd"));
    // The guards of large members only, unchecked; every member behind a guard, unchecked; and
    // every skip run anyway and compared (the default of these tests).
    let envs: [&[(&str, &str)]; 3] = [
        &[("YAVS_CYCLE_GUARD", ""), ("YAVS_CYCLE_SKIP_CHECK", "")],
        &[("YAVS_CYCLE_SKIP_CHECK", "")],
        &[],
    ];
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_skip");
        assert_eq!(normal.len(), 68, "{normal:?}");
        // Hand-checked at 1 ns: `q` counted once at the edge at 0 ns, nothing of `w` set.
        assert!(
            normal[0].contains("cnt=00") && normal[0].contains(" q=03 "),
            "{normal:?}"
        );
        for env in envs {
            let mut env = env.to_vec();
            env.push(("YAVS_CYCLE_STATS", "1"));
            let (cycle, err) = run_vars(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_skip", &env);
            assert_eq!(cycle, normal, "{t:?} {env:?}\n{err}");
            assert!(
                err.contains("cycle mode: 14 of 18 processes run cycle-based"),
                "{err}"
            );
            for name in [
                "pc", "pf", "pg", "wr", "pm", "la", "lb", "pr", "po", "c1", "c2", "px", "pd", "pp",
            ] {
                let (section, _) = row(&err, &format!("cycle_skip.{name}"));
                assert_eq!(section, "cycle-based:", "{name}\n{err}");
            }
            // Skips happened: the guards saw unchanged inputs.
            let skipped: u64 = err
                .lines()
                .find_map(|l| {
                    l.split_once(" runs, ")?
                        .1
                        .strip_suffix(" skipped")?
                        .parse()
                        .ok()
                })
                .unwrap_or_else(|| panic!("no guard counts\n{err}"));
            // Fewer than before the settle at an edge was marked: a step outside the edge's cone
            // does not reach its guard.
            assert!(skipped > 0, "{skipped}\n{err}");
        }
    }
    // The failed check of a member whose last run saw what its early run saw (no confirming
    // pass of the loop group, which would find it in a debug build).
    for env in [
        &[
            ("YAVS_CYCLE_GUARD", ""),
            ("YAVS_CYCLE_SKIP_CHECK", ""),
            ("YAVS_CYCLE_CHECKS", "on"),
        ][..],
        &[("YAVS_CYCLE_CHECKS", "on")],
    ] {
        for mode in [None, Some("--cycle")] {
            let mut args = vec![
                "run",
                "--work=work:lib",
                "--no-cache",
                "--tier=interp",
                "-gFAIL=true",
            ];
            args.extend(mode);
            args.push("cycle_skip");
            let o = yavs_env(&dir, &args, env);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "{mode:?}: {err}");
            assert!(
                err.contains("error[E3009]: index 9 outside of")
                    && err.contains("at 142 ns, delta 2, in process cycle_skip.la"),
                "{mode:?}: {err}"
            );
        }
    }
}

/// The settle at an edge runs only what the clocked members of the classes that had one, and the
/// inputs read, reach (`edgecone.vhd`): clocks with logic of their own, logic two clocks share,
/// edges in one delta, a gated clock, a register that keeps its value, a memory written on one
/// clock and read into another's logic and outside, resets made by a register of another clock and
/// from outside, reports of two clocks in one delta (in process order). Against the normal mode at
/// widths 1 to 128: with the tests' checks, with every settle
/// compared with one of every step (`YAVS_CYCLE_SKIP_CHECK=full`), and with neither.
#[test]
fn an_edge_settles_only_what_the_classes_that_had_one_reach() {
    let dir = tempdir("cycle-edgecone");
    // Every step in a function of its own (behind a guard) and nothing checked, so a step skipped
    // wrongly gives another result; the tests' checks (every run compared with one of every step);
    // those with every check of the code; the default guards and functions.
    let envs: [&[(&str, &str)]; 4] = [
        &[("YAVS_CYCLE_SKIP_CHECK", "")],
        &[],
        &[
            ("YAVS_CYCLE_SKIP_CHECK", "full"),
            ("YAVS_CYCLE_CHECKS", "all"),
        ],
        &[("YAVS_CYCLE_GUARD", ""), ("YAVS_CYCLE_SKIP_CHECK", "")],
    ];
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("edgecone{w}.vhd"),
            &with_width(include_str!("cycle/edgecone.vhd"), w),
        );
        // Hand-checked at 4.5 ns, after the first edge of ca: ra counted once, za and sa all ones
        // (`not ra` of 0, plus yb of 0).
        let hex = |v: u128| format!("{v:0n$X}", n = (w as usize).div_ceil(4));
        let ones = if w == 128 { u128::MAX } else { (1 << w) - 1 };
        let want = format!(" a={} {} {} ", hex(1), hex(ones), hex(ones));
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_edgecone");
            let mon: Vec<&String> = normal.iter().filter(|l| l.contains("t=")).collect();
            assert_eq!(mon.len(), 120, "{w} {t:?} {normal:?}");
            assert!(mon[4].contains(&want), "{want}\n{}", mon[4]);
            assert!(normal.len() > 130, "{w}: the reports of pna, pnc and pnd");
            for env in envs {
                let mut env = env.to_vec();
                env.push(("YAVS_CYCLE_STATS", "1"));
                let (cycle, err) = run_vars(
                    &dir,
                    &[&t[..], &["--cycle"]].concat(),
                    "cycle_edgecone",
                    &env,
                );
                assert_eq!(cycle, normal, "width {w} {t:?} {env:?}\n{err}");
                // Steps were left out at edges (with a step per function).
                let (all, ran) = edge_settle(&err);
                let packed = env.contains(&("YAVS_CYCLE_GUARD", ""));
                assert!(ran < all || packed, "{all} {ran} {env:?}\n{err}");
                assert!(
                    err.contains("cycle mode: 26 of 30 processes run cycle-based"),
                    "{w}: {err}"
                );
                for name in [
                    "pa", "pb", "psa", "psb", "pk", "pkq", "pq", "pg", "pw", "pr", "psy", "ph",
                    "ph2", "ppa", "ppb", "pna", "pnc", "pnd", "pw2",
                ] {
                    let (section, _) = row(&err, &format!("cycle_edgecone.{name}"));
                    assert_eq!(section, "cycle-based:", "{name}\n{err}");
                }
            }
        }
    }
}

/// From `YAVS_CYCLE_STATS`: the settle's operations at the wakes with an edge, and those the mark
/// let run.
fn edge_settle(err: &str) -> (u64, u64) {
    let l = (err.lines())
        .find_map(|l| l.strip_prefix("cycle stats: wakes with an edge: settle operations "))
        .unwrap_or_else(|| panic!("no settle counts\n{err}"));
    let (a, b) = l.split_once(", ").expect("two counts");
    let b = b.split_once(' ').expect("a count").0;
    (a.parse().expect("a number"), b.parse().expect("a number"))
}

/// Expression actuals of ports (a logic expression, a record aggregate, an integer and a vector
/// expression) and array aggregates without a context (a range choice of a subtype, choices
/// against the direction of the index subtype), against the normal mode.
#[test]
fn port_expression_actuals_and_aggregates_without_context_equal_the_normal_mode() {
    let dir = tempdir("cycle-portact");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("portact{w}.vhd"),
            &with_width(include_str!("cycle/portact.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_portact");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_portact");
            assert_eq!(normal.len(), 30, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // Every process but the testbench's `main`: the units, their eight actuals, the tracker.
            assert!(
                err.contains("cycle mode: 14 of 15 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A memory of more than 2**20 elements, written and read at run-time indices, in the region.
#[test]
fn a_memory_over_a_million_elements_equals_the_normal_mode() {
    let dir = tempdir("cycle-bigram");
    analyze(&dir, "bigram.vhd", include_str!("cycle/bigram.vhd"));
    let (normal, _) = run(&dir, &[], "cycle_bigram");
    let (cycle, err) = run(&dir, &["--cycle"], "cycle_bigram");
    assert_eq!(normal.len(), 40, "{normal:?}");
    assert_eq!(cycle, normal, "{err}");
    assert!(
        err.contains("cycle mode: 1 of 2 processes run cycle-based"),
        "{err}"
    );
}

/// CPU idioms: `'pos` and `'val` of an enumeration and of logic types, `+` and `-` of a vector and
/// a `std_ulogic`, an aggregate as an operand of `&`, a function on an unconstrained vector given
/// the result of `resize`; against the normal mode.
#[test]
fn cpu_idioms_equal_the_normal_mode() {
    let dir = tempdir("cycle-idioms");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("idioms{w}.vhd"),
            &with_width(include_str!("cycle/idioms.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_idioms");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_idioms");
            assert_eq!(normal.len(), 30, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Arrays of no elements (a generic of 0): an array of null elements written at a run-time
/// index and read through `resize`, a record with a null field, null values in `&`, a null
/// variable and constant join; one indexed by a `natural` stays out; against the normal mode.
#[test]
fn null_arrays_equal_the_normal_mode() {
    let dir = tempdir("cycle-nullvec");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("nullvec{w}.vhd"),
            &with_width(include_str!("cycle/nullvec.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_nullvec");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_nullvec");
            assert_eq!(normal.len(), 20, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 6 of 8 processes run cycle-based")
                    && err.contains(&format!(
                        "cycle_nullvec.u.wr2 (nullvec{w}.vhd:40)         an index chosen at run \
                         time of an array of null elements (nullvec{w}.vhd:43)"
                    )),
                "{w}: {err}"
            );
        }
    }
}

/// An `out` port of `std_logic_vector` on a `std_ulogic_vector` actual: its writers join when
/// no element of the formal has two drivers (a register, a concurrent assignment, a member reading
/// the actual); a formal with two drivers stays resolved in the event kernel; against the normal
/// mode at widths 1 to 128.
#[test]
fn resolved_out_ports_on_unresolved_actuals_equal_the_normal_mode() {
    let dir = tempdir("cycle-portres");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("portres{w}.vhd"),
            &with_width(include_str!("cycle/portres.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_portres");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_portres");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert!(normal.iter().any(|l| l.contains('X')), "{normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 6 processes run cycle-based")
                    && err.contains("E4124 cycle_portres.u.two is also driven by"),
                "{w}: {err}"
            );
        }
    }
}

/// The actual of an `in` port with a conversion (not collapsed, the kernel updates the formal)
/// driven by members (a register, a combinational process): the drivers join, and the readers
/// of the formal see its values in the same delta; widths 1 to 128.
#[test]
fn actuals_of_converted_in_ports_driven_by_members_equal_the_normal_mode() {
    let dir = tempdir("cycle-portin");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("portin{w}.vhd"),
            &with_width(include_str!("cycle/portin.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_portin");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_portin");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 5 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Elements of elements, both chosen at run time, assigned: vectors of both directions, a slice of
/// each element, a field of an array of records, a variable; an inner and an outer index out of
/// range fail with the normal mode's error; widths 1 to 128.
#[test]
fn elements_of_elements_at_run_time_indices_equal_the_normal_mode() {
    let dir = tempdir("cycle-index2");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("index2{w}.vhd"),
            &with_width(include_str!("cycle/index2.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_index2");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_index2");
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
        for (bad, msg) in [
            ("1", format!("outside of range {} downto 0 for m", w - 1)),
            ("2", "index 7 outside of range 0 to 5 for m".to_string()),
        ] {
            let mut shown = Vec::new();
            for mode in [None, Some("--cycle")] {
                let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
                args.extend(mode);
                let g = format!("-gBAD={bad}");
                args.extend(["cycle_index2", g.as_str()]);
                let o = yavs(&dir, &args);
                let err = String::from_utf8_lossy(&o.stderr).to_string();
                assert_eq!(o.status.code(), Some(1), "{w} {bad} {mode:?}: {err}");
                assert!(err.contains(&msg), "{w} {bad} {mode:?}: {err}");
                shown.push(err[err.find("error[").expect("an error")..].to_string());
            }
            assert_eq!(shown[0], shown[1], "{w} {bad}");
        }
    }
}

/// A decoder ROM: a constant table of records with enumeration, integer, logic and vector fields
/// indexed by an enumeration chosen at run time, assigned whole; widths 1 to 128.
#[test]
fn a_constant_table_of_records_read_at_run_time_equals_the_normal_mode() {
    let dir = tempdir("cycle-romrec");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("romrec{w}.vhd"),
            &with_width(include_str!("cycle/romrec.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_romrec");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_romrec");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Aggregates with slice pieces: with `others`, two pieces and an element, both directions, in a
/// register; widths 1 to 128.
#[test]
fn aggregates_with_slice_pieces_equal_the_normal_mode() {
    let dir = tempdir("cycle-slicepc");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("slicepc{w}.vhd"),
            &with_width(include_str!("cycle/slicepc.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_slicepc");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_slicepc");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 5 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A register table as a generic array of records: a loop over its range assigns slices whose
/// lengths are fields of its elements (the table named for its range and read by field); widths
/// 1 to 128.
#[test]
fn a_generic_table_of_records_equals_the_normal_mode() {
    let dir = tempdir("cycle-regtab");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("regtab{w}.vhd"),
            &with_width(include_str!("cycle/regtab.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_regtab");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_regtab");
            assert_eq!(normal.len(), 10, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Sizes from `math_real` in functions of the design (`natural(ceil(log2(real(n))))`), folded
/// where the arguments are known, also a vector result through a function with an assert
/// (`to_size`): every design process joins; widths 1 to 128.
#[test]
fn math_real_sizes_of_constants_fold() {
    let dir = tempdir("cycle-mathreal");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("mathreal{w}.vhd"),
            &with_width(include_str!("cycle/mathreal.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_mathreal");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_mathreal");
            assert_eq!(normal.len(), 10, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 4 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// The index ranges of logical operator results given to a function that walks its argument:
/// `std_logic_1164`'s, `numeric_std`'s and the predefined ones of BIT vectors; widths 1 to 128.
#[test]
fn ranges_of_logical_operator_results_equal_the_normal_mode() {
    let dir = tempdir("cycle-oprange");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("oprange{w}.vhd"),
            &with_width(include_str!("cycle/oprange.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_oprange");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_oprange");
            assert_eq!(normal.len(), 10, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Attributes of an element of a memory chosen at run time (`mem(r)'low`), as slice bounds in an
/// unrolled loop and in arithmetic; widths 1 to 128.
#[test]
fn attributes_of_elements_chosen_at_run_time_equal_the_normal_mode() {
    let dir = tempdir("cycle-elemattr");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("elemattr{w}.vhd"),
            &with_width(include_str!("cycle/elemattr.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_elemattr");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_elemattr");
            assert_eq!(normal.len(), 10, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Parts of elements chosen at run time (`elempart.vhd`): the low byte and all but the top bit of an
/// element read, the low byte of another written, where elements start on words (64, 128) and where
/// they do not (65).
#[test]
fn parts_of_elements_chosen_at_run_time_equal_the_normal_mode() {
    let dir = tempdir("cycle-elempart");
    for w in [64, 65, 128] {
        analyze(
            &dir,
            &format!("elempart{w}.vhd"),
            &with_width(include_str!("cycle/elempart.vhd"), w),
        );
        let (normal, _) = run(&dir, &["--tier=interp"], "cycle_elempart");
        let flags = ["--tier=interp", "--cycle"];
        let (cycle, err) = run(&dir, &flags, "cycle_elempart");
        assert_eq!(normal.len(), 12, "{w} {normal:?}");
        assert_eq!(cycle, normal, "width {w}\n{err}");
        assert!(
            err.contains("cycle mode: 3 of 4 processes run cycle-based"),
            "{w}: {err}"
        );
    }
}

/// Records of scalars known during elaboration given to an inlined function (a fixed-point
/// format): a local sized by a function of one, a local format from an aggregate and from a
/// function returning a record, one chosen by comparing records, an assert on them; widths 1 to
/// 128 (the process stayed out with "a value that is not known during elaboration").
#[test]
fn records_of_scalars_given_to_a_function_fold() {
    let dir = tempdir("cycle-recfmt");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("recfmt{w}.vhd"),
            &with_width(include_str!("cycle/recfmt.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_recfmt");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_recfmt");
            assert_eq!(normal.len(), 8, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Narrow lanes of packed vectors (`lanes.vhd`): delay chains, lanes copied in place and from two
/// sources in turn, in-place shifts of a variable both ways, a swap, sign-extended lanes multiplied,
/// at lane widths 1 to 128; also with one and two words in registers (`YAVS_CYCLE_REGS`) and with
/// every check (lanes moved by one word op must equal those moved one by one).
#[test]
fn lanes_moved_word_by_word_equal_the_normal_mode() {
    let dir = tempdir("cycle-lanes");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("lanes{w}.vhd"),
            &with_width(include_str!("cycle/lanes.vhd"), w),
        );
        let (normal, _) = run(&dir, &["--tier=interp"], "cycle_lanes");
        assert_eq!(normal.len(), 16, "{w} {normal:?}");
        let flags = ["--tier=interp", "--cycle"];
        for env in [
            &[][..],
            &[("YAVS_CYCLE_REGS", "1")],
            &[("YAVS_CYCLE_REGS", "2")],
            &[("YAVS_CYCLE_CHECKS", "all")],
        ] {
            let (cycle, err) = run_vars(&dir, &flags, "cycle_lanes", env);
            assert_eq!(cycle, normal, "width {w} {env:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A format record made by an aggregate whose field leaves its subtype (`s` is 2 in `natural
/// range 0 to 1`) is not folded: the process stays in the kernel and fails with the normal
/// mode's message, at the `return` of the function that made it.
#[test]
fn a_record_field_outside_its_subtype_fails_as_in_the_normal_mode() {
    let dir = tempdir("cycle-recfmt-range");
    let good = "return (s => 1, f => a.f";
    let text = include_str!("cycle/recfmt.vhd");
    assert!(text.contains(good));
    for w in [1, 63, 64, 65, 128] {
        let bad = with_width(&text.replace(good, "return (s => a.s + 1, f => a.f"), w);
        analyze(&dir, &format!("recfmtr{w}.vhd"), &bad);
        for t in tiers() {
            let mut shown = Vec::new();
            for mode in [None, Some("--cycle")] {
                let mut args = vec!["run", "--work=work:lib", "--no-cache"];
                args.extend(&t);
                args.extend(mode);
                args.push("cycle_recfmt");
                let o = yavs(&dir, &args);
                let err = String::from_utf8_lossy(&o.stderr).into_owned();
                assert_eq!(o.status.code(), Some(1), "{w} {t:?} {mode:?}: {err}");
                assert!(
                    err.contains("error[E3009]: value 2 outside of INTEGER range 0 to 1"),
                    "{w} {t:?} {mode:?}: {err}"
                );
                if mode.is_some() {
                    assert!(
                        err.contains("cycle mode: 0 of 2 processes run cycle-based"),
                        "{w} {t:?}: {err}"
                    );
                }
                shown.push(err[err.find("error[").expect("an error")..].to_string());
            }
            assert_eq!(shown[0], shown[1], "{w} {t:?}");
        }
    }
}

/// Clocks whose edges come in a known delta of their time step (made with a delay: the first;
/// by clock processes waiting for a time: the second) share an edge group with logic between
/// their registers, logic of the second kind's registers sampled by the first kind's joins, and
/// the other direction stays out; against the normal mode at widths 1 to 128. A clock process
/// whose timeout elaboration does not know (here 0 ns every other time) keeps its own group.
#[test]
fn clocks_of_a_known_delta_share_their_logic() {
    let dir = tempdir("cycle-deltacls");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("deltacls{w}.vhd"),
            &with_width(include_str!("cycle/deltacls.vhd"), w),
        );
        for t in tiers() {
            // Width 1 truncates in `numeric_std`: the warnings of members are not made (A6).
            let k = |v: Vec<String>| -> Vec<String> {
                v.into_iter().filter(|l| l.contains("k=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_deltacls");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_deltacls");
            let (normal, cycle) = (k(normal), k(cycle));
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 12 of 19 processes run cycle-based")
                    && err.contains(
                        "it computes, from registers on cycle_deltacls.ca, a value a register on \
                         cycle_deltacls.cc samples"
                    ),
                "{w}: {err}"
            );
        }
    }
    let (normal, _) = run(&dir, &[], "cycle_deltazero");
    let (cycle, err) = run(&dir, &["--cycle"], "cycle_deltazero");
    assert_eq!(normal.len(), 1, "{normal:?}");
    assert_eq!(cycle, normal, "{err}");
    assert!(
        err.contains("cycle mode: 2 of 7 processes run cycle-based")
            && err.contains(
                "it computes, from registers on cycle_deltazero.cc, a value a register on \
                 cycle_deltazero.cz samples"
            ),
        "{err}"
    );
}

/// `numeric_std` shifts and rotations by a count chosen at run time, of any width (a byte
/// rotation of a vector of more than 64 elements was not implemented), against the normal mode at
/// widths 1 to 128, counts from 0 past twice the width and 1000.
#[test]
fn shifts_by_a_count_chosen_at_run_time_equal_the_normal_mode() {
    let dir = tempdir("cycle-wideshift");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("wideshift{w}.vhd"),
            &with_width(include_str!("cycle/wideshift.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_wideshift");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_wideshift");
            assert_eq!(normal.len() as u32, 2 * w + 4, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Processes outside the region whose result depends on the deltas their inputs change in (a
/// variable's value of the previous run; a signal the sensitivity list does not name; state in a
/// process rejected for a `'Z'` first, with a sensitivity list or a wait), fed by a register and
/// combinational logic: the logic in their fan-in stays in the event kernel with the reason, the
/// register joins, and so does logic read only by a concurrent assignment rejected for its `'Z'`
/// or by a process that acts at clock edges alone, or behind one; logic read by an asynchronous
/// reset branch that its list does not name, or by a latch written as feedback, stays out;
/// against the normal mode at widths 1 to 128 (with the logic in the region their outputs
/// differed).
#[test]
fn logic_feeding_a_process_that_counts_its_runs_stays_out() {
    let dir = tempdir("cycle-runcount");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("runcount{w}.vhd"),
            &with_width(include_str!("cycle/runcount.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_runcount");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_runcount");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 26 processes run cycle-based")
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.c2, in the fan-in of \
                         cycle_runcount.stale"
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.c3, in the fan-in of \
                         cycle_runcount.dut.list"
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.e4, in the fan-in of \
                         cycle_runcount.dut.mask"
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.c6, in the fan-in of \
                         cycle_runcount.dut.waiter"
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.c8, in the fan-in of \
                         cycle_runcount.dut.resets"
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.c12, in the fan-in of \
                         cycle_runcount.dut.li <= ..."
                    )
                    && err.contains(
                        "it computes, combinationally, cycle_runcount.dut.c14, in the fan-in of \
                         cycle_runcount.dut.li2 <= ..."
                    ),
                "{w}: {err}"
            );
            for m in ["e3", "c7", "c9"] {
                let label = format!("cycle_runcount.dut.{m} <= ...");
                assert_eq!(row(&err, &label).0, "cycle-based:", "{err}");
            }
        }
    }
}

/// Slices bounded by variables of known values (fields packed by `lo` and `hi`, an element
/// written at `lo` in a loop of known iterations) and a branch chosen by a function elaboration
/// evaluates join the region; a variable given another value on each path, or read by an
/// `elsif` after a branch changed it, leaves an element stale and its process out; against the
/// normal mode at widths 1 to 128.
#[test]
fn slices_bounded_by_known_variables_join() {
    let dir = tempdir("cycle-knownvar");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("knownvar{w}.vhd"),
            &with_width(include_str!("cycle/knownvar.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_knownvar");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_knownvar");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 5 of 9 processes run cycle-based"),
                "{w}: {err}"
            );
            for p in ["two", "elsy", "wl"] {
                let (_, r) = row(&err, &format!("cycle_knownvar.{p}"));
                assert!(r.contains("  E4120 "), "{r}");
            }
        }
    }
}

/// A pipeline of integers shifted by concatenating an element and a slice of an array of
/// integers, both orders; stages 1 to 128.
#[test]
fn concatenations_of_integer_elements_equal_the_normal_mode() {
    let dir = tempdir("cycle-intcat");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("intcat{w}.vhd"),
            &with_width(include_str!("cycle/intcat.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_intcat");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_intcat");
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Memories held in shared variables (A15): single-process RAMs that read before and after they
/// write, both at the address written and at another on one edge, join with their initial content;
/// a RAM two processes write and read stays in the event kernel with the reason; against the
/// normal mode at widths 1 to 128.
#[test]
fn shared_variable_memories_equal_the_normal_mode() {
    let dir = tempdir("cycle-shram");
    for w in [1, 63, 64, 65, 128] {
        let file = format!("shram{w}.vhd");
        std::fs::write(
            dir.join(&file),
            with_width(include_str!("cycle/shram.vhd"), w),
        )
        .expect("write design");
        let o = yavs(
            &dir,
            &[
                "analyze",
                "--std=2008",
                "--relaxed",
                "--work=work:lib",
                &file,
            ],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        for t in tiers() {
            let flags = [&t[..], &["--relaxed"]].concat();
            let (normal, _) = run(&dir, &flags, "cycle_shram");
            let (cycle, err) = run(&dir, &[&flags[..], &["--cycle"]].concat(), "cycle_shram");
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // The two single-process RAMs; not the two processes of `tp`, nor those of `hid`
            // (an alias, a function), nor `main`.
            assert!(
                err.contains("cycle mode: 2 of 9 processes run cycle-based"),
                "{w}: {err}"
            );
            for (who, other) in [("mem", "peek"), ("mem2", "via_fn")] {
                assert!(
                    err.contains(&format!(
                        "E4114 the shared variable `{who}` is also used by cycle_shram.hid.{other}"
                    )),
                    "{who}: {err}"
                );
            }
            assert!(
                err.contains(
                    "E4114 the shared variable `mem` is also used by cycle_shram.tp.p_read"
                ),
                "{err}"
            );
        }
    }
}

/// A `while` loop over a string generic in a function a clocked process tests: elaboration
/// evaluates the call per process, region code folds it; against the normal mode.
#[test]
fn calls_elaboration_evaluated_per_process_equal_the_normal_mode() {
    let dir = tempdir("cycle-strgen");
    analyze(&dir, "strgen.vhd", include_str!("cycle/strgen.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_strgen");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_strgen");
        assert_eq!(normal.len(), 6, "{t:?} {normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 3 processes run cycle-based"),
            "{err}"
        );
    }
}

/// The report records of a run's standard output: severity and message without the time, and
/// the process line under it.
fn records(out: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut lines = out.lines().peekable();
    while let Some(l) = lines.next() {
        let Some(rest) = l.strip_prefix("** ") else {
            continue;
        };
        let (sev, msg) = rest.split_once(": ").expect("a report line");
        let msg = msg.split_once(": ").map_or(msg, |(_, m)| m);
        let at = match lines.peek() {
            Some(p) if p.starts_with("    Process") => lines.next().unwrap_or_default(),
            _ => "",
        };
        v.push((format!("{sev}: {msg}"), at.to_string()));
    }
    v
}

/// Asserts and reports of pure functions whose calls fold with constants (A22, `foldrep.vhd`):
/// one that holds prints nothing; a note or warning (an assert, a report, in a call inside a
/// call, the same call in two processes, the second from the elaboration's call cache, a library
/// warning; a constrained vector result, a width check with a message constant, one in an inlined
/// call and one only elaboration's value folds, all holding, join) is made
/// once per process at 0 fs where the normal mode makes it at every call, with the same text and
/// process, also from the compilation cache; one of severity error or failure keeps its process
/// in the event kernel, so its messages, times and the exit status are the normal mode's; a call
/// whose arguments only the unrolled loop knows joins when its assert holds and stays out when it
/// fails; a failing call in a branch that never runs is not seen. A constant's report is made
/// once in both modes. The library warning is dropped also when a constant's call of the normal
/// mode made the call cache entry.
#[test]
fn reports_of_calls_folded_with_constants_are_made() {
    let dir = tempdir("cycle-foldrep");
    analyze(&dir, "foldrep.vhd", include_str!("cycle/foldrep.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib"];
        if !flags.contains(&"--cache") {
            args.push("--no-cache");
        }
        args.extend(flags.iter().filter(|f| **f != "--cache"));
        args.push("cycle_foldrep");
        let o = yavs(&dir, &args);
        let out: String = String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| match l.split_once("fs+") {
                Some((a, b)) => format!(
                    "{a}fs{}\n",
                    b.trim_start_matches(|c: char| c.is_ascii_digit())
                ),
                None => format!("{l}\n"),
            })
            .collect();
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        (o.status.code(), out, err)
    };
    let members = |err: &str, n: u32| {
        assert!(
            err.contains(&format!("cycle mode: {n} of 11 processes run cycle-based")),
            "{n}: {err}"
        )
    };
    let report = |gens: &[&str]| {
        let mut f = gens.to_vec();
        f.push("--cycle");
        with_report(&dir, &f, "cycle_foldrep", String::new())
    };
    let sorted = |v: &[(String, String)]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let res = |o: &str| {
        o.lines()
            .filter(|l| l.contains("a="))
            .map(String::from)
            .collect::<Vec<_>>()
    };
    for t in tiers() {
        let flags = |gens: &[&'static str], cycle: bool| {
            let mut f = [&t[..], gens].concat();
            if cycle {
                f.push("--cycle");
            }
            f
        };
        // Every assert holds: the same output.
        let normal = go(&flags(&["-gg=4"], false));
        let (c, out, err) = go(&flags(&["-gg=4"], true));
        assert_eq!((c, &out), (normal.0, &normal.1), "{t:?}\n{err}");
        assert_eq!(records(&out).len(), 2, "{out}");
        members(&err, 9);
        // The constant's report, on the standard error once in both modes.
        for e in [&normal.2, &err] {
            assert_eq!(e.matches("(init): say 1\n").count(), 1, "{t:?}\n{e}");
        }
        // Failing asserts of notes and warnings: once per process, the same records; a second
        // run with the compilation cache gives the first's output; a library warning of a
        // folded call is dropped (A23), the normal mode prints it.
        // First the cached library warning: a later cached run of cycle mode (`trunc` false
        // folds the call too) would make the entry itself.
        for (s, extra) in [
            ("-gs=warning", "-gtrunc=true --cache"),
            ("-gs=note", ""),
            ("-gs=warning", ""),
            ("-gs=warning", "--cache"),
            ("-gs=warning", "-gtrunc=true"),
        ] {
            let gens: Vec<&str> = ["-gg=5", s]
                .into_iter()
                .chain(extra.split_whitespace())
                .collect();
            let (nc, nout, _) = go(&flags(&gens, false));
            let (c, out, err) = go(&flags(&gens, true));
            let lib = "NUMERIC_STD.TO_UNSIGNED: vector truncated";
            if extra.contains("-gtrunc=true") {
                assert_eq!(out.matches(lib).count(), 0, "{t:?} {extra}\n{out}");
                assert!(nout.contains(lib), "{t:?}\n{nout}");
            }
            if extra.contains("--cache") {
                let again = go(&flags(&gens, true));
                assert_eq!((again.0, &again.1), (c, &out), "{t:?} warm\n{}", again.2);
            }
            assert_eq!(c, nc, "{t:?} {s}\n{err}");
            let (n, cy) = (records(&nout), records(&out));
            let n: Vec<(String, String)> = n.into_iter().filter(|r| !r.0.contains(lib)).collect();
            assert_eq!(sorted(&cy), sorted(&n), "{t:?} {s}\n{nout}\n{out}");
            assert_eq!(sorted(&cy).len(), cy.len(), "{t:?} {s}: made twice\n{out}");
            assert!(
                n.len() > cy.len(),
                "{t:?} {s}: the normal mode reports at every call"
            );
            for (p, n) in [("pa", 5), ("pb", 7), ("pd", 5)] {
                assert!(
                    out.contains(&format!(
                        "0fs: half of odd {n}\n    Process cycle_foldrep.u.{p}\n"
                    )),
                    "{t:?} {s} {p}\n{out}"
                );
            }
            assert_eq!(res(&out), res(&nout), "{t:?} {s}");
            members(&err, 9);
        }
        // Error and failure: the processes calling `half` stay out; the normal mode's output.
        for s in ["-gs=error", "-gs=failure"] {
            let normal = go(&flags(&["-gg=5", s], false));
            let (c, out, err) = go(&flags(&["-gg=5", s], true));
            assert_eq!((c, &out), (normal.0, &normal.1), "{t:?} {s}\n{err}");
            assert_eq!(c, Some(1), "{out}");
            members(&err, 6);
        }
        // A failing assert in the unrolled loop keeps `pc` out.
        for s in ["-gs=note", "-gs=warning"] {
            let normal = go(&flags(&["-gloop_odd=true", s], false));
            let (c, out, err) = go(&flags(&["-gloop_odd=true", s], true));
            assert_eq!((c, &out), (normal.0, &normal.1), "{t:?} {s}\n{err}");
            members(&err, 8);
        }
        // The failing call in `pa`'s branch on `dead`: elaboration's failure is not seen while
        // the branch cannot run (above), and when it runs `pa` stays out.
        for o in [&normal.1, &normal.2, &out, &err] {
            assert!(!o.contains("half of odd 1"), "{o}");
        }
        let normal = go(&flags(&["-gdead=true"], false));
        let (c, out, err) = go(&flags(&["-gdead=true"], true));
        assert_eq!((c, &out), (normal.0, &normal.1), "{t:?}\n{err}");
        assert!(out.contains("Failure: 5000000fs: half of odd 1"), "{out}");
        members(&err, 8);
    }
    let r = report(&["-gg=5", "-gs=error"]);
    for (p, f) in [("pa", "half"), ("pb", "outer"), ("pd", "half")] {
        assert!(
            row(&r, &format!("cycle_foldrep.u.{p}"))
                .1
                .contains(&format!(
                    "a call of `{f}` that reports with severity error when it is evaluated"
                )),
            "{p}: {r}"
        );
    }
    let r = report(&["-gloop_odd=true"]);
    assert!(
        row(&r, "cycle_foldrep.u.pc").1.contains(
            "an assert or report of `half`, called with arguments known during elaboration, \
             that cycle mode cannot show to hold"
        ),
        "{r}"
    );
}

/// Out ports of a subtype resolved by a function of the design on an unresolved actual
/// (`ownres.vhd`): the function runs in both modes, so the actual is the inverse of the register,
/// also for a function named `resolved` (once taken for the library's); the drivers stay out of
/// the region (another source).
#[test]
fn a_resolution_function_of_the_design_runs_on_out_ports() {
    let dir = tempdir("cycle-ownres");
    analyze(&dir, "ownres.vhd", include_str!("cycle/ownres.vhd"));
    let want = ["q=1110", "q=1100", "q=1000", "q=0000", "q=0001", "q=0011"];
    for top in ["cycle_ownres_other", "cycle_ownres_named"] {
        for t in tiers() {
            let (normal, _) = run(&dir, &t, top);
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), top);
            let got: Vec<&str> = (normal.iter())
                .map(|l| l.rsplit(": ").next().unwrap_or(""))
                .collect();
            assert_eq!(got, want, "{top} {t:?}");
            assert_eq!(cycle, normal, "{top} {t:?}\n{err}");
            assert!(
                err.contains("has a resolution function of its own"),
                "{top}: {err}"
            );
        }
    }
}

/// The reports of constants' initial values are made once under `--cycle`, as in the normal
/// mode, with and without the call cache: the value passes of cycle mode evaluate constants
/// apart from the model and neither print nor count what they report (they printed every
/// report a second time). A process calling a function that reports an error stays out.
#[test]
fn reports_of_constants_are_made_once() {
    let dir = tempdir("cycle-constrep");
    analyze(&dir, "constrep.vhd", include_str!("cycle/constrep.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("cycle_constrep");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        let init: Vec<String> = (err.lines())
            .filter(|l| l.contains("(init)"))
            .map(String::from)
            .collect();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            init,
            err,
        )
    };
    for t in tiers() {
        for cache in [&["--no-cache"][..], &[]] {
            let f = [&t[..], cache].concat();
            let normal = go(&f);
            assert_eq!(normal.2.len(), 9, "{:?}", normal.2);
            let cycle = go(&[&f[..], &["--cycle"]].concat());
            let members = "cycle mode: 2 of 5 processes run cycle-based";
            assert!(cycle.3.contains(members), "{}", cycle.3);
            assert_eq!(
                (&cycle.0, &cycle.1, &cycle.2),
                (&normal.0, &normal.1, &normal.2),
                "{f:?}\n{}",
                cycle.3
            );
            // The member-only instance's constant fails: the run stops in both modes.
            let normal = go(&[&f[..], &["-gbad=true"]].concat());
            let cycle = go(&[&f[..], &["-gbad=true", "--cycle"]].concat());
            assert!(normal.3.contains("leaf constant fails"), "{}", normal.3);
            assert_ne!(normal.0, Some(0), "{}", normal.3);
            assert_eq!((&cycle.0, &cycle.2), (&normal.0, &normal.2), "{}", cycle.3);
        }
    }
}

/// A constant whose value needs an impure call on the way, left to the run by elaboration: the
/// value pass of cycle mode undoes that call (the package state is made again), so the run
/// makes it once.
#[test]
fn impure_calls_of_the_value_pass_are_undone() {
    let dir = tempdir("cycle-undo");
    std::fs::write(dir.join("undo.vhd"), include_str!("cycle/undo.vhd")).expect("write design");
    let o = yavs(
        &dir,
        &[
            "analyze",
            "--std=2008",
            "--relaxed",
            "--work=work:lib",
            "undo.vhd",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for t in tiers() {
        for cycle in [false, true] {
            let mut args = vec!["run", "--relaxed", "--work=work:lib", "--no-cache"];
            args.extend(&t);
            if cycle {
                args.push("--cycle");
            }
            args.push("cycle_undo");
            let o = yavs(&dir, &args);
            let (out, err) = (
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr),
            );
            assert!(
                out.contains("D=4 C=14 q=14 next=2"),
                "{args:?}\n{out}\n{err}"
            );
            if cycle {
                assert!(
                    err.contains("cycle mode: 0 of 3 processes run cycle-based"),
                    "{err}"
                );
            }
        }
    }
}

/// An input nothing drives that starts with a metavalue: the normal mode reads it for the whole
/// run, the region reads 0 (the contract), so the results may differ. Cycle mode names such an
/// input at the start and in the report; a signal with a start value of its own (`w`) or a driver
/// is not named.
#[test]
fn undriven_inputs_with_a_metavalue_are_named() {
    let dir = tempdir("cycle-undriven");
    analyze(&dir, "undriven.vhd", include_str!("cycle/undriven.vhd"));
    let notice = "1 input driven by nothing read as 0 (see --cycle-report)";
    for t in tiers() {
        for drive in [false, true] {
            let g = format!("-gdrive={drive}");
            let mut args = vec!["run", "--work=work:lib", "--no-cache", &g];
            args.extend(&t);
            let normal = yavs(&dir, &[&args[..], &["cycle_undriven"]].concat());
            let cycle = yavs(&dir, &[&args[..], &["--cycle", "cycle_undriven"]].concat());
            let report = yavs(
                &dir,
                &[&args[..], &["--cycle-report", "cycle_undriven"]].concat(),
            );
            let (n, c) = (
                String::from_utf8_lossy(&normal.stdout),
                String::from_utf8_lossy(&cycle.stdout),
            );
            let (err, rep) = (
                String::from_utf8_lossy(&cycle.stderr),
                String::from_utf8_lossy(&report.stderr),
            );
            let members = if drive { "2 of 4" } else { "1 of 3" };
            let head = format!("cycle mode: {members} processes run cycle-based");
            assert!(err.contains(&head), "{err}");
            assert_eq!(err.contains(notice), !drive, "{err}");
            assert_eq!(rep.contains("    cycle_undriven.v\n"), !drive, "{rep}");
            assert!(!rep.contains("cycle_undriven.w"), "{rep}");
            let q = if drive { "q='1'" } else { "q='0'" };
            assert!(n.contains("q='1'") && c.contains(q), "{n}\n{c}");
        }
    }
}

/// A member that would hide a metavalue the testbench reads stays in the event kernel: one that
/// assigns a metavalue literal to a signal a process outside reads, and one that passes on a
/// 'Z' an outside driver may give. The same literal on a signal only members read, and a copy of
/// a signal without 'Z', stay members. Widths 1 to 128.
#[test]
fn metavalues_the_testbench_reads_are_kept() {
    let dir = tempdir("cycle-metaout");
    analyze(&dir, "metaout.vhd", include_str!("cycle/metaout.vhd"));
    for t in tiers() {
        for w in [1, 63, 64, 65, 128] {
            let g = format!("-gW={w}");
            let mut args = vec!["run", "--work=work:lib", "--no-cache", &g];
            args.extend(&t);
            let normal = yavs(&dir, &[&args[..], &["cycle_metaout"]].concat());
            let cycle = yavs(&dir, &[&args[..], &["--cycle", "cycle_metaout"]].concat());
            let report = yavs(
                &dir,
                &[&args[..], &["--cycle-report", "cycle_metaout"]].concat(),
            );
            let (n, c) = (
                String::from_utf8_lossy(&normal.stdout),
                String::from_utf8_lossy(&cycle.stdout),
            );
            let rep = String::from_utf8_lossy(&report.stderr);
            assert!(
                n.contains("pin='Z'") && n.contains("pin3='Z'") && n.contains("x=true"),
                "{n}"
            );
            assert_eq!(
                (cycle.status.code(), &c),
                (normal.status.code(), &n),
                "{w} {t:?}\n{rep}"
            );
            assert!(
                rep.contains("cycle mode: 3 of 10 processes run cycle-based"),
                "{rep}"
            );
            for p in [
                "cycle_metaout.copy ",
                "cycle_metaout.copy3 ",
                "cycle_metaout.drv ",
            ] {
                assert!(
                    rep.lines().any(|l| l.trim_start().starts_with(p)),
                    "{p}: {rep}"
                );
            }
        }
    }
}

/// The shift operators of `numeric_std` and `std_logic_1164` (`srl` of `signed` logical, a
/// negative count known here the other way, bits of results by index in their own ranges) join,
/// in a combinational process and a register, against the normal mode at widths 1 to 128 and
/// counts from 0 past twice the width; one whose count may be negative at run time stays out.
#[test]
fn shift_operators_equal_the_normal_mode() {
    let dir = tempdir("cycle-shiftop");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("shiftop{w}.vhd"),
            &with_width(include_str!("cycle/shiftop.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_shiftop");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_shiftop");
            assert_eq!(normal.len() as u32, 2 * w + 4, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 4 processes run cycle-based")
                    && err.contains("may be negative"),
                "{w}: {err}"
            );
        }
    }
}

/// Qualified record aggregates (`r_t'(...)`, a nested one, as a register's value and an operand
/// of `=`) join, against the normal mode at widths 1 to 128; one whose integer field may leave
/// its subtype stays out, and where it does (generic `fail`) both modes stop with one error.
#[test]
fn qualified_record_aggregates_equal_the_normal_mode() {
    let dir = tempdir("cycle-qualrec");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("qualrec{w}.vhd"),
            &with_width(include_str!("cycle/qualrec.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_qualrec");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_qualrec");
            assert_eq!(normal.len(), 21, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 5 processes run cycle-based")
                    && err.contains("may lie outside their subtypes"),
                "{w}: {err}"
            );
            let mut shown = Vec::new();
            for mode in [None, Some("--cycle")] {
                let mut args = vec!["run", "--work=work:lib", "--no-cache", "-gfail=true"];
                args.extend(&t);
                args.extend(mode);
                args.push("cycle_qualrec");
                let o = yavs(&dir, &args);
                let err = String::from_utf8_lossy(&o.stderr).into_owned();
                assert_eq!(o.status.code(), Some(1), "{w} {t:?} {mode:?}: {err}");
                shown.push(err[err.find("error[").expect("an error")..].to_string());
            }
            assert_eq!(shown[0], shown[1], "{w} {t:?}");
        }
    }
}

/// Conditions elaboration evaluated whose arguments the folds do not know themselves prune their
/// branch in the subset checker and the lowering alike, so their processes join (the lowering
/// read the pruned branch and the cross-check kept them out), against the normal mode at widths
/// 1 to 128.
#[test]
fn conditions_elaboration_evaluated_prune_alike() {
    let dir = tempdir("cycle-elabcall");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("elabcall{w}.vhd"),
            &with_width(include_str!("cycle/elabcall.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_elabcall");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_elabcall");
            assert_eq!(normal.len(), 10, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A `numeric_std` shift function given a negative count known during elaboration stops the
/// normal mode at the call; its process stays out and both modes stop with one error, at widths 1
/// to 128 (cycle mode shifted the other way and exited 0). With a count of 1 it joins.
#[test]
fn a_negative_known_count_of_a_shift_function_fails_as_in_the_normal_mode() {
    let dir = tempdir("cycle-shiftneg");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("shiftneg{w}.vhd"),
            &with_width(include_str!("cycle/shiftneg.vhd"), w),
        );
        for t in tiers() {
            let mut shown = Vec::new();
            for mode in [None, Some("--cycle")] {
                let mut args = vec!["run", "--work=work:lib", "--no-cache"];
                args.extend(&t);
                args.extend(mode);
                args.push("cycle_shiftneg");
                let o = yavs(&dir, &args);
                let err = String::from_utf8_lossy(&o.stderr).into_owned();
                assert_eq!(o.status.code(), Some(1), "{w} {t:?} {mode:?}: {err}");
                assert!(
                    err.contains("value -1 outside of NATURAL range"),
                    "{w} {t:?} {mode:?}: {err}"
                );
                shown.push(err[err.find("error[").expect("an error")..].to_string());
            }
            assert_eq!(shown[0], shown[1], "{w} {t:?}");
            let one = [&t[..], &["-gk=1"]].concat();
            let (normal, _) = run(&dir, &one, "cycle_shiftneg");
            let (cycle, err) = run(&dir, &[&one[..], &["--cycle"]].concat(), "cycle_shiftneg");
            assert_eq!((normal.len(), &cycle), (1, &normal), "{w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A register outside the region on a clock a member makes (a divider) samples a combinational
/// output of the region in the delta the clock edge arrives: the normal mode's new value comes a
/// delta later, so that logic stays in the event kernel (found by the fuzzer). A register output
/// sampled the same way is right in both modes.
#[test]
fn a_register_on_a_clock_the_region_makes_samples_its_logic_before_it_settles() {
    let dir = tempdir("cycle-dclkreg");
    analyze(&dir, "dclkreg.vhd", include_str!("cycle/dclkreg.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_dclkreg");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_dclkreg");
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 5 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// A register outside the region on a clock copied twice samples combinational logic fed by a
/// member register on the once-copied clock: the normal mode's new value comes a delta after
/// the edge. The register stays out only for a later rule (an output port associated in parts),
/// so the run-count cone is computed again then and keeps that logic out (found by the fuzzer).
#[test]
fn a_register_left_out_late_still_keeps_the_logic_it_samples_out() {
    let dir = tempdir("cycle-latecone");
    analyze(&dir, "latecone.vhd", include_str!("cycle/latecone.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_latecone");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_latecone");
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 7 processes run cycle-based")
                    && err.contains("in the fan-in of cycle_latecone.u.process"),
                "{w}: {err}"
            );
        }
    }
}

/// A register with an asynchronous reset outside the region (the checker fits it, the lowering
/// takes no `while` loop) on a copy of the clock samples logic fed by a member register on the
/// clock: it is a delta seed like a plain register on such a clock (found by the fuzzer).
#[test]
fn a_register_with_a_reset_on_a_late_clock_keeps_the_logic_it_samples_out() {
    let dir = tempdir("cycle-lateasync");
    analyze(&dir, "lateasync.vhd", include_str!("cycle/lateasync.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_lateasync");
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_lateasync");
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 6 processes run cycle-based")
                    && err.contains("in the fan-in of cycle_lateasync.late"),
                "{w}: {err}"
            );
        }
    }
}
