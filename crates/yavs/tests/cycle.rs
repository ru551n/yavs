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

/// `err` with each failed check's message as plain `--cycle` gives it (A34): without the
/// operands' values, which only runs without `--two-state` keep.
fn bare(err: &str) -> String {
    const HINT: &str = " (run without --cycle to see the values)";
    let one = |l: &str| -> String {
        let Some(m) = l.trim_start().strip_prefix("error[E3009]: ") else {
            return l.to_string();
        };
        let head = &l[..l.len() - m.len()];
        let w: Vec<&str> = m.splitn(4, ' ').collect();
        let m = match &w[..] {
            // The type is not in the normal mode's message: INTEGER in these tests.
            ["division", "by", "zero"] => {
                "division by zero, or a result that cannot be represented as INTEGER".into()
            }
            ["result", "of", _, rest] => {
                let (op, ty) = rest
                    .split_once(' ')
                    .and_then(|(op, r)| Some((op, r.split_once(" cannot be represented as ")?.1)))
                    .expect("an arithmetic message");
                if ["/", "mod", "rem"].contains(&op) {
                    format!("division by zero, or a result that cannot be represented as {ty}")
                } else {
                    format!("result of {op} cannot be represented as {ty}")
                }
            }
            [k @ ("value" | "index"), _, "outside", rest] => format!("{k} outside {rest}"),
            _ => panic!("a check message bare() does not know: {m}"),
        };
        format!("{head}{m}{HINT}")
    };
    err.lines().map(one).collect::<Vec<_>>().join("\n")
        + if err.ends_with('\n') { "\n" } else { "" }
}

fn analyze(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).expect("write design");
    let o = yavs(dir, &["analyze", "--std=2008", "--work=work:lib", file]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// `err`, the standard error of a run with `flags`, followed by the region report of the same
/// design when it ran in cycle mode (a `--cycle-report` run, which analyses and stops): the
/// membership rows are read there, a cycle run prints one line.
fn with_report(dir: &Path, flags: &[&str], top: &str, err: String) -> String {
    with_report_env(dir, flags, top, err, &[])
}

/// [`with_report`] with the environment of the run.
fn with_report_env(
    dir: &Path,
    flags: &[&str],
    top: &str,
    mut err: String,
    env: &[(&str, &str)],
) -> String {
    if flags
        .iter()
        .any(|f| matches!(*f, "--cycle" | "--cycle-strict"))
    {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend((flags.iter()).filter(|f| !f.starts_with("--cycle")));
        args.extend(["--cycle-report", top]);
        let o = yavs_env(dir, &args, env);
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
    let err = with_report_env(dir, flags, top, err, env);
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
                &[&t[..], &["-gCYCLES=2000", mode, "--two-state"]].concat(),
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_partial",
        );
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
                && err.contains("E4101 a `while` loop or a loop without a scheme (partial.vhd:19)"),
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_quiet",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_export",
        );
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
            let (cycle, err) = run(d, &[&t[..], &["--cycle", "--two-state"]].concat(), top);
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_localpkg",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_xcopy",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_lazy",
        );
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
            "--two-state",
        ])
        .arg("cycle_lazy")
        .output()
        .expect("run yavs");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("q=0 q2=0 tq=4 handshakes=0"), "{out}");
}

/// An input that reaches a register only through an asynchronous reset made inside the region,
/// the register read only by another register: a pulse between edges resets it, so it is settled
/// at once (it was left for the next edge, which saw the pulse over).
#[test]
fn a_reset_pulse_between_edges_through_combinational_code_resets_the_register() {
    let dir = tempdir("cycle-lazyrst");
    analyze(&dir, "lazyrst.vhd", include_str!("cycle/lazyrst.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_lazyrst");
        assert_eq!(normal.len(), 6, "{normal:?}");
        assert!(normal[2].ends_with("21000000fs+0: s=0"), "{normal:?}");
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_lazyrst",
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 3 of 5 processes run cycle-based")
                && err.contains(
                    "wakes: 1 settled at once, through combinational code to what outside sees \
                     (cycle_lazyrst.ext to the reset branch of cycle_lazyrst.areg)"
                ),
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &[mode, "--two-state"]].concat(),
                "cycle_ints",
            );
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
    let (cycle, err) = run(
        &dir,
        &["--tier=interp", "--cycle", "--two-state"],
        "cycle_part_list",
    );
    assert!(
        normal.len() == 1 && normal[0].ends_with("w=01"),
        "{normal:?}"
    );
    assert_eq!(cycle, normal);
    // Element by element: `half` reads `v(1)`, which its list does not name.
    assert!(
        err.contains("0 of 2 processes run cycle-based")
            && err.contains(
                "E4121 an incomplete sensitivity list: the process reads cycle_part_list.v(1), which the list does not name"
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
            "--two-state",
            "cycle_partial",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(o.stdout.is_empty(), "no simulation runs");
    // The RTL process below the top level that cannot join is listed; the testbench is exempt.
    assert!(
        err.contains("error[E4101]: a `while` loop or a loop without a scheme is not supported")
            && err.contains("= note: in process cycle_partial.dut.pick (partial.vhd:"),
        "{err}"
    );
    // At the loop in the function, and the call in the process.
    assert!(
        err.contains("partial.vhd:19:5") && err.contains("- called from here"),
        "{err}"
    );
    // The concurrent `q <= r` joins.
    assert!(!err.contains("cycle_partial.dut.q <= ..."), "{err}");
    assert!(!err.contains("cycle_partial.main"), "{err}");
}

#[test]
fn cycle_mode_refuses_the_options_that_see_inside_the_region() {
    let dir = tempdir("cycle-not-with");
    analyze(&dir, "partial.vhd", include_str!("cycle/partial.vhd"));
    for (opt, name) in [
        ("--wave=x.fst --wave-variables", "--wave-variables"),
        ("--cov=s", "--cov"),
        ("--vhpi=nothing", "--vhpi"),
        ("--activity", "--activity"),
        ("--activity=x.json", "--activity"),
    ] {
        for mode in ["--cycle", "--cycle-strict"] {
            let mut args = vec!["run", "--work=work:lib", mode, "--two-state"];
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
            &[
                "run",
                "--work=work:lib",
                "--cycle",
                "--two-state",
                opt,
                "cycle_partial",
            ],
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{opt}: {err}");
        assert!(!err.contains("E4102"), "{opt}: {err}");
    }
}

/// A misspelt `--two-state` names it, and the removed `--cycle-exact-meta` is an unknown option.
#[test]
fn two_state_spelling_and_removed_exact_meta() {
    let dir = tempdir("cycle-twostate-flag");
    let o = yavs(
        &dir,
        &["run", "--work=work:lib", "--cycle", "--two-stat", "top"],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("`--two-state`"), "{err}");
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle",
            "--cycle-exact-meta",
            "top",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("--cycle-exact-meta"), "{err}");
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
    let o = report(&["--cycle-report", "--two-state"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success() && o.stdout.is_empty(), "{err}");
    assert!(
        err.starts_with("cycle mode: 5 of 7 processes run cycle-based")
            && err.contains("cycle_partial.dut.pick (partial.vhd:66)"),
        "{err}"
    );
    // With a file the report goes there.
    let file = dir.join("report.txt");
    let o = report(&[&format!("--cycle-report={}", file.display()), "--two-state"]);
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
            "--two-state",
            "cycle_partial",
        ],
    );
    // One line, then the count of metavalues the region read as 0, which is its own line; `reg`
    // evaluates its failure assert, so nothing warns about it.
    let err = String::from_utf8_lossy(&o.stderr);
    let mut lines = err.lines();
    assert_eq!(
        lines.next(),
        Some(
            "cycle mode: 5 of 7 processes run cycle-based, 2 in the event kernel (see --cycle-report)"
        ),
        "{err}"
    );
    assert!(
        lines.all(|l| l.contains("metavalue elements read as 0")),
        "{err}"
    );
}

/// A52, A57: an assert cycle mode cannot evaluate (a severity known only at run time) keeps its
/// process in the event kernel, so the failure ends the run as in the normal mode; under
/// `--cycle-strict` it is error E4140 at the statement. `--no-asserts` drops the process's own
/// asserts and reports in both modes (a note counts them, the report lists them once each) and the
/// process joins; the assert of the function it calls still runs.
#[test]
fn an_assert_cycle_mode_cannot_evaluate_keeps_its_process_out_and_no_asserts_drops_it() {
    let dir = tempdir("cycle-awarn");
    analyze(&dir, "awarn.vhd", include_str!("cycle/awarn.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--tier=interp", "--no-cache"];
        args.extend(flags);
        args.push("cycle_awarn");
        let o = yavs(&dir, &args);
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let (rc, _, normal) = go(&[]);
    assert!(
        rc == Some(1) && normal.contains("the design ends its own test"),
        "{normal}"
    );
    let (rc, err, out) = go(&["--cycle", "--two-state"]);
    assert_eq!((rc, &out), (Some(1), &normal), "{err}");
    assert!(
        err.starts_with("cycle mode: 2 of 4 processes run cycle-based"),
        "{err}"
    );
    let (rc, err, _) = go(&["--cycle-strict", "--two-state"]);
    assert_eq!(rc, Some(2), "{err}");
    assert!(
        err.contains(
            "error[E4140]: an assert or report of this form is not supported in cycle mode\n  \
             --> "
        ) && err.contains("awarn.vhd:36:7")
            && err.contains("a severity that is not known during elaboration")
            && err.contains("or run with `--no-asserts` to drop the asserts and reports below")
            && err.contains("in process cycle_awarn.dut.reg (awarn.vhd:23)"),
        "{err}"
    );
    let note = "note: `--no-asserts`: 10 assert and report statements in 2 processes below the \
                top-level unit are not evaluated";
    let (rc, err, dropped) = go(&["--no-asserts"]);
    assert!(
        rc == Some(0) && dropped.contains("end q=3C") && !dropped.contains("ends its own test"),
        "{dropped}"
    );
    assert!(err.starts_with(note), "{err}");
    for flags in [
        &["--no-asserts", "--cycle", "--two-state"][..],
        &["--no-asserts", "--cycle-strict", "--two-state"],
    ] {
        let (rc, err, out) = go(flags);
        assert_eq!((rc, &out), (Some(0), &dropped), "{flags:?}: {err}");
        assert!(err.starts_with(note), "{err}");
        assert!(
            err.contains("cycle mode: 3 of 4 processes run cycle-based"),
            "{err}"
        );
    }
    let (rc, report, _) = go(&["--no-asserts", "--cycle-report", "--two-state"]);
    assert_eq!(rc, Some(0), "{report}");
    assert!(
        report.contains(
            "  10 asserts and reports not evaluated in cycle mode, held by 2 cycle-based processes:
       9  cycle_awarn.dut.reg (awarn.vhd:23)
       1  cycle_awarn.dut.assert (awarn.vhd:43)
  each assert and report not evaluated, by severity (2 failure, 1 known at run time, 5 error, 1 warning, 1 note):
    failure   awarn.vhd:29  cycle_awarn.dut.reg
    failure   awarn.vhd:27  cycle_awarn.dut.reg
    run-time  awarn.vhd:36  cycle_awarn.dut.reg
    error     awarn.vhd:35  cycle_awarn.dut.reg
    error     awarn.vhd:31  cycle_awarn.dut.reg
    error     awarn.vhd:32  cycle_awarn.dut.reg
    error     awarn.vhd:38  cycle_awarn.dut.reg
    error     awarn.vhd:43  cycle_awarn.dut.assert (awarn.vhd:43)
    warning   awarn.vhd:33  cycle_awarn.dut.reg
    note      awarn.vhd:34  cycle_awarn.dut.reg
"
        ) && report.contains("those below it are dropped (--no-asserts)"),
        "{report}"
    );
    // W4101 is retired, and `-Werror=W4101` still accepted.
    let (rc, err, out) = go(&["--cycle", "--two-state", "-Werror=W4101"]);
    assert_eq!((rc, &out), (Some(1), &normal), "{err}");
    assert!(!err.contains("W4101"), "{err}");
}

/// The run of `asserts.vhd` with `flags` at width `w`: exit status, standard output, standard error.
fn asserts_run(dir: &Path, flags: &[&str]) -> (Option<i32>, String, String) {
    let mut args = vec!["run", "--work=work:lib", "--no-cache"];
    args.extend(flags);
    args.push("cycle_asserts");
    let o = yavs(dir, &args);
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// The reports of a run as a sorted list of (time, severity, message, process), the delta left out:
/// cycle mode gives each in the delta of the region's wake and in its own order (A52).
fn report_set(out: &str) -> Vec<String> {
    let lines: Vec<&str> = out.lines().collect();
    let mut v: Vec<String> = (lines.windows(2))
        .filter(|w| w[0].starts_with("** "))
        .map(|w| {
            let head = w[0]
                .split_once("fs+")
                .map_or(w[0].to_string(), |(t, rest)| {
                    format!("{t}fs{}", rest.split_once(':').map_or("", |x| x.1))
                });
            format!("{head} | {}", w[1].trim())
        })
        .collect();
    v.sort();
    v
}

/// A52: the asserts and reports below the top-level unit, evaluated in cycle mode, give the
/// normal mode's messages, times, severities and processes and stop the run where it stops: a
/// clocked member at its edge, combinational members once their inputs are settled and only when
/// an element they wait on changed (a part of a vector across a word boundary, one field of a
/// record, an integer, the same value assigned again), a concurrent assert. Membership asserted:
/// `wide` (a sensitivity list naming more than it reads) and `rst_chk` (a report in its reset
/// branch) stay out with E4140.
#[test]
fn asserts_below_the_top_are_evaluated_as_in_the_normal_mode() {
    for w in [63, 64, 65, 128] {
        let dir = tempdir("cycle-asserts");
        analyze(
            &dir,
            "asserts.vhd",
            &with_width(include_str!("cycle/asserts.vhd"), w),
        );
        let (rc, normal, _) = asserts_run(&dir, &["--tier=interp"]);
        assert_eq!(rc, Some(1), "{normal}");
        assert!(
            normal.contains("the design ends the test at 00001110")
                && normal.contains("f1 is 0011")
                && normal.contains("i above 299: 400"),
            "{normal}"
        );
        let want = report_set(&normal);
        assert_eq!(want.len(), 29, "{w}: {want:#?}");
        for t in tiers() {
            for exact in [false, true] {
                let flags = if exact {
                    vec!["--cycle"]
                } else {
                    vec!["--cycle", "--two-state"]
                };
                let mode = flags.join(" ");
                let (rc, out, err) = asserts_run(&dir, &[&t[..], &flags].concat());
                assert_eq!(rc, Some(1), "{w} {t:?} {mode}: {err}");
                assert_eq!(report_set(&out), want, "{w} {t:?} {mode}\n{out}");
                // Under the exact rules a member with notes reads only what members drive:
                // `reg` reads the testbench's `a`, and its readers then read `n` from outside.
                let n = if exact { 3 } else { 11 };
                assert!(
                    err.contains(&format!("cycle mode: {n} of 15 processes run cycle-based")),
                    "{err}"
                );
            }
        }
        let err = with_report(
            &dir,
            &["--cycle", "--two-state"],
            "cycle_asserts",
            String::new(),
        );
        for p in [
            "reg",
            "early",
            "peer",
            "sum",
            "field",
            "part",
            "stp",
            "int",
            "assert (asserts.vhd:107)",
        ] {
            assert!(
                err.contains(&format!("    cycle_asserts.dut.{p}")),
                "{p}: {err}"
            );
        }
        let exact = with_report(&dir, &["--cycle"], "cycle_asserts", String::new());
        for (p, l, what) in [
            ("sum", 79, "cycle_asserts.a"),
            ("early", 41, "cycle_asserts.dut.n"),
        ] {
            assert!(
                exact.contains(&format!("    cycle_asserts.dut.{p} (asserts.vhd:{l})")) && exact.contains(&format!(
                    "E4140 an assert or report without `--two-state` reading a signal the region does not drive alone: {what} (asserts.vhd:{l})"
                )),
                "{p}: {exact}"
            );
        }
        assert!(
            err.contains("E4140 an assert or report in a process whose sensitivity list names more than it reads: it does not read cycle_asserts.b (asserts.vhd:109)")
                && err.contains("E4140 an assert or report of this form: an assert or report in the asynchronous reset branch of a clocked process (asserts.vhd:118)"),
            "{err}"
        );
        // `--exit-severity=error`: the error at 90 ns ends both runs there.
        let (rc, normal, _) = asserts_run(&dir, &["--exit-severity=error"]);
        let (rc2, cycle, err) =
            asserts_run(&dir, &["--exit-severity=error", "--cycle", "--two-state"]);
        assert_eq!((rc, rc2), (Some(1), Some(1)), "{err}");
        assert!(
            normal.ends_with("nine, an error\n    Process cycle_asserts.dut.reg\n"),
            "{normal}"
        );
        assert_eq!(report_set(&cycle), report_set(&normal), "{cycle}");
    }
}

/// A52: a combinational member whose report reads what depends on its own output runs in a loop
/// group after what makes it; its runs before the last report nothing, so cycle mode reports the
/// settled value once per change, the normal mode's last report of each time step.
#[test]
fn a_combinational_report_in_a_loop_group_reports_settled_values_only() {
    let dir = tempdir("cycle-assertloop");
    analyze(&dir, "assertloop.vhd", include_str!("cycle/assertloop.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_assertloop");
        let o = yavs(&dir, &args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        // (time, message) of each report, the delta left out.
        (out.lines())
            .filter_map(|l| l.strip_prefix("** Note: "))
            .filter_map(|l| {
                let (t, rest) = l.split_once("fs+")?;
                Some((t.to_string(), rest.split_once(": ")?.1.to_string()))
            })
            .collect::<Vec<_>>()
    };
    let mut settled: Vec<(String, String)> = Vec::new();
    for r in go(&[]) {
        if settled.last().is_some_and(|p| p.0 == r.0) {
            settled.pop();
        }
        settled.push(r);
    }
    assert_eq!(settled.len(), 4, "{settled:?}");
    for t in tiers() {
        assert_eq!(
            go(&[&t[..], &["--cycle", "--two-state"]].concat()),
            settled,
            "{t:?}"
        );
    }
    let err = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "cycle_assertloop",
        String::new(),
    );
    assert!(
        err.contains("in the region: a loop group of 2 processes, 3 process runs per settle")
            && err.contains("cycle mode: 2 of 3 processes run cycle-based"),
        "{err}"
    );
}

/// A52: a concurrent `assert not (<edge> and a and b)` runs on the kernel's edge trigger and joins
/// as a clocked member checking `a and b` on the values before the edge (one of the testbench
/// reading a member's output too); `d` set in the delta of the edge is seen as by the normal mode's
/// process. The `or` form stays out (E4140).
#[test]
fn an_edge_tested_concurrent_assert_is_a_check_at_the_edge() {
    for w in [1, 63, 64, 65, 128] {
        let dir = tempdir("cycle-edgeassert");
        analyze(
            &dir,
            "edgeassert.vhd",
            &with_width(include_str!("cycle/edgeassert.vhd"), w),
        );
        let go = |flags: &[&str]| {
            let mut args = vec!["run", "--work=work:lib", "--no-cache"];
            args.extend(flags);
            args.push("cycle_edgeassert");
            let o = yavs(&dir, &args);
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let normal = go(&[]);
        let want = report_set(&normal);
        assert_eq!(want.len(), if w == 1 { 32 } else { 7 }, "{w}: {normal}");
        for t in tiers() {
            assert_eq!(
                report_set(&go(&[&t[..], &["--cycle", "--two-state"]].concat())),
                want,
                "{w} {t:?}"
            );
        }
        let err = with_report(
            &dir,
            &["--cycle", "--two-state"],
            "cycle_edgeassert",
            String::new(),
        );
        assert!(
            err.contains("cycle mode: 5 of 7 processes run cycle-based")
                && [23, 26, 48].iter().all(|l| err.lines().any(|x| {
                    x.contains(&format!("assert (edgeassert.vhd:{l})")) && x.ends_with(" clocked")
                }))
                && err.contains("E4140 an assert or report of this form: a call of `rising_edge` (edgeassert.vhd:29)"),
            "{w}: {err}"
        );
    }
}

/// Without `--two-state` a member below the top-level unit whose report reads a signal the
/// region does not drive alone stays out (E4140) and reports the 'U' start as the normal mode;
/// under the two-state contract it joins and reports 0. The top-level checker that reads only a
/// member's register joins and reports its 'U' start as 'U' without `--two-state` (the second
/// image), as 0 under the two-state contract.
#[test]
fn exact_meta_keeps_out_a_report_reading_the_kernel() {
    let dir = tempdir("cycle-exactnote");
    analyze(&dir, "exactnote.vhd", include_str!("cycle/exactnote.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_exactnote");
        let o = yavs(&dir, &args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        report_set(&String::from_utf8_lossy(&o.stdout))
    };
    let at0 = |r: &[String]| -> Vec<String> {
        (r.iter())
            .filter(|l| l.starts_with("** Note: 0fs"))
            .cloned()
            .collect()
    };
    let normal = go(&[]);
    assert_eq!(normal.len(), 6, "{normal:#?}");
    let has = |r: &[String], t: &str| at0(r).iter().any(|l| l.contains(t));
    assert!(
        has(&normal, "q=U r=U") && has(&normal, "top q=U"),
        "{normal:#?}"
    );
    for t in tiers() {
        let exact = go(&[&t[..], &["--cycle"]].concat());
        assert!(
            has(&exact, "q=U r=U") && has(&exact, "top q=U"),
            "{t:?} {exact:#?}"
        );
        assert_eq!(exact, normal, "{t:?}");
        let plain = go(&[&t[..], &["--cycle", "--two-state"]].concat());
        assert!(
            has(&plain, "q=0 r=0") && has(&plain, "top q=0"),
            "{t:?} {plain:#?}"
        );
    }
    let err = with_report(&dir, &["--cycle"], "cycle_exactnote", String::new());
    assert!(
        err.contains("cycle mode: 3 of 5 processes run cycle-based")
            && err.contains("E4140 an assert or report without `--two-state` reading a signal the region does not drive alone: cycle_exactnote.d (exactnote.vhd:16)"),
        "{err}"
    );
    let err = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "cycle_exactnote",
        String::new(),
    );
    assert!(
        err.contains("cycle mode: 4 of 5 processes run cycle-based"),
        "{err}"
    );
}

/// A59: without `--two-state` asserts and reports of members read the second image: a 'U' or
/// 'X' start (whole, in a field, in a part of a vector; widths 1/63/64/65/128; `to_hstring` digits
/// `X`) prints as in the normal mode from a clocked checker and from a combinational process; a BIT
/// copy of a 'U' prints 0. Differences: `zero` fires on the region's image only (two-state text, a
/// line the normal mode does not make), `one` on the second image only (not made), and the
/// combinational report of the event from 'U' to 0 at 0 fs is gone (two-state twins). Membership
/// asserted: every process but the stimulus joins in both contracts.
#[test]
fn exact_meta_notes_show_the_second_images_metavalues() {
    let dir = tempdir("cycle-notemeta");
    analyze(&dir, "notemeta.vhd", include_str!("cycle/notemeta.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_notemeta");
        let o = yavs(&dir, &args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        report_set(&String::from_utf8_lossy(&o.stdout))
    };
    let normal = go(&[]);
    let at = |r: &[String], time: &str, t: &str| {
        (r.iter()).any(|l| l.starts_with(&format!("** Note: {time}fs ")) && l.contains(t))
    };
    for t in [
        "q1=U qx='X' q63=UUU",
        "q64=XXXXXXXXXXXXXXXX q65=UUU",
        "0XXXXXXXXXXX000",
        "ph=XX000000000000XXX00000000000000X po=00X0",
        "r.a=UUU",
        "r.b=U",
        "comb s1=U sx=X s65=UUU",
    ] {
        assert!(at(&normal, "0", t), "{t}: {normal:#?}");
    }
    assert!(at(&normal, "10000000", "bv=0000"), "{normal:#?}");
    let zero = "** Note: 0fs zero q64=0000000000000000 | Process cycle_notemeta.chk";
    let comb = format!(
        "** Note: 0fs comb s1=0 sx=0 s65={} | Process cycle_notemeta.dut.comb",
        "0".repeat(65)
    );
    for t in tiers() {
        let exact = go(&[&t[..], &["--cycle"]].concat());
        let extra: Vec<&String> = exact.iter().filter(|l| !normal.contains(l)).collect();
        let lost: Vec<&String> = normal.iter().filter(|l| !exact.contains(l)).collect();
        assert!(
            extra == [zero] && lost == [&comb],
            "{t:?} {extra:#?} {lost:#?}"
        );
        let plain = go(&[&t[..], &["--cycle", "--two-state"]].concat());
        assert!(
            at(&plain, "0", "q1=0 qx='0' q63=000") && at(&plain, "0", "comb s1=0 sx=0 s65=000"),
            "{t:?} {plain:#?}"
        );
    }
    for flags in [&["--cycle"][..], &["--cycle", "--two-state"]] {
        let err = with_report(&dir, flags, "cycle_notemeta", String::new());
        assert!(
            err.contains("cycle mode: 11 of 12 processes run cycle-based"),
            "{flags:?} {err}"
        );
    }
}

/// A25: the reports of clocked members of two edge classes whose edges come in one delta are
/// reported in elaboration order, as the normal mode's processes of that delta run, not class by
/// class as the region makes them.
#[test]
fn reports_of_two_edge_classes_come_in_elaboration_order() {
    let dir = tempdir("cycle-noteorder");
    analyze(&dir, "noteorder.vhd", include_str!("cycle/noteorder.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_noteorder");
        let o = yavs(&dir, &args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let normal = go(&[]);
    assert_eq!(normal.matches("** Note").count(), 12, "{normal}");
    for t in tiers() {
        assert_eq!(
            go(&[&t[..], &["--cycle", "--two-state"]].concat()),
            normal,
            "{t:?}"
        );
    }
    let err = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "cycle_noteorder",
        String::new(),
    );
    assert!(
        err.contains("cycle mode: 4 of 5 processes run cycle-based"),
        "{err}"
    );
}

/// An assert that holds by what the lowering knows (a width check on variables of constants in a
/// called function) is no code: no check of its combinational process for an input to reach, so
/// the input waits for the next edge.
#[test]
fn an_assert_that_holds_by_known_values_is_no_check() {
    let dir = tempdir("cycle-foldassert");
    analyze(&dir, "foldassert.vhd", include_str!("cycle/foldassert.vhd"));
    let err = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "cycle_foldassert",
        String::new(),
    );
    assert!(
        err.contains("cycle mode: 2 of 3 processes run cycle-based")
            && err.contains("  wakes: 0 settled at once"),
        "{err}"
    );
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle",
            "--two-state",
            "cycle_foldassert",
        ],
    );
    assert!(
        o.status.success() && String::from_utf8_lossy(&o.stdout).contains("q=04"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// The range check of a port's expression actual in the region names the port and points at the
/// actual, as the normal mode's does.
#[test]
fn a_port_actual_check_names_the_port_as_the_normal_mode() {
    let dir = tempdir("cycle-portcheck");
    analyze(&dir, "portcheck.vhd", include_str!("cycle/portcheck.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_portcheck");
        let o = yavs(&dir, &args);
        assert_eq!(o.status.code(), Some(1));
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    let normal = go(&[]);
    let place = "portcheck.vhd:21:73";
    assert!(
        normal.contains("for port N") && normal.contains(place),
        "{normal}"
    );
    let exact = go(&["--cycle"]);
    assert!(
        exact.contains("value 4 outside of INTEGER range 0 to 3 for port N")
            && exact.contains(place),
        "{exact}"
    );
    let plain = go(&["--cycle", "--two-state"]);
    assert!(
        plain.contains("for port N") && plain.contains(place),
        "{plain}"
    );
}

/// A52: the idle rule skips an edge run that would change nothing, but never one that makes a
/// note: a member that holds its state and reports at every edge reports 40 times.
#[test]
fn an_edge_run_that_makes_a_note_is_never_idle() {
    let dir = tempdir("cycle-idlenote");
    analyze(&dir, "idlenote.vhd", include_str!("cycle/idlenote.vhd"));
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib", "--no-cache"];
        args.extend(flags);
        args.push("cycle_idlenote");
        let o = yavs_env(
            &dir,
            &args,
            &[("YAVS_CYCLE_SKIP_CHECK", ""), ("YAVS_CYCLE_STATS", "1")],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    let (normal, _) = go(&[]);
    assert_eq!(normal.matches("held is '1'").count(), 40, "{normal}");
    for t in tiers() {
        let (cycle, err) = go(&[&t[..], &["--cycle", "--two-state"]].concat());
        assert_eq!(cycle, normal, "{t:?}");
        assert!(
            err.contains("cycle stats: edge runs skipped as idle: 0"),
            "{err}"
        );
    }
}

/// A54: `--no-asserts` drops the asserts and reports below the top-level unit in both modes, also
/// from a work library whose cache holds the run without it, and back (the option is in the keys).
#[test]
fn no_asserts_drops_them_below_the_top_in_both_modes_and_is_in_the_cache_keys() {
    let dir = tempdir("cycle-noasserts");
    analyze(
        &dir,
        "asserts.vhd",
        &with_width(include_str!("cycle/asserts.vhd"), 65),
    );
    let note = "note: `--no-asserts`: 15 assert and report statements in 11 processes below the \
                top-level unit are not evaluated";
    let go = |flags: &[&str]| {
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("cycle_asserts");
        let o = yavs(&dir, &args);
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    for _ in 0..2 {
        for mode in [
            &[][..],
            &["--cycle", "--two-state"],
            &["--cycle-strict", "--two-state"],
        ] {
            let (rc, out, err) = go(&[mode, &["--no-asserts"]].concat());
            assert_eq!(
                (rc, out.as_str()),
                (
                    Some(0),
                    "** Note: 110000000fs+1: chk sees 11\n    Process cycle_asserts.dut.reg\n"
                ),
                "{mode:?}: {err}"
            );
            assert!(err.starts_with(note), "{mode:?}: {err}");
            if !mode.is_empty() {
                // `wide` and `rst_chk` join once their reports are dropped.
                assert!(
                    err.contains("cycle mode: 13 of 15 processes run cycle-based"),
                    "{err}"
                );
            }
            if mode != ["--cycle-strict", "--two-state"] {
                let (rc, out, err) = go(mode);
                assert_eq!(rc, Some(1), "{mode:?}: {err}");
                assert!(out.contains("the design ends the test"), "{out}");
                assert!(!err.contains("--no-asserts"), "{err}");
            }
        }
    }
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
            "--two-state",
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
    assert!(counts[0].1.starts_with("E4129 "), "{counts:?}");
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_ops",
            );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_codes",
        );
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
entity cycle_div is
end entity;
architecture a of cycle_div is
  signal clk : bit := '0';
  signal x : integer := 3;
  signal y : integer := 0;
begin
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      x <= x - 1;
      y <= 100 / x;
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

/// The normal mode, plain `--cycle` (no values in a failed check's message) and the exact
/// contract (no `--two-state`: the normal mode's message).
const MODES: [&[&str]; 3] = [&[], &["--cycle", "--two-state"], &["--cycle"]];

/// `top` fails with error `msg` and `note` in the normal mode and in cycle mode, with one of its
/// two processes in the region: the same diagnostic, with the place of the statement and of the
/// calls around it; plain `--cycle` without the operands' values (A34).
fn same_failure(dir: &Path, top: &str, msg: &str, note: &str) {
    let mut shown = Vec::new();
    for mode in MODES {
        let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
        args.extend(mode);
        args.push(top);
        let o = yavs(dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{top} {mode:?}: {err}");
        let want = if mode.contains(&"--two-state") {
            bare(msg)
        } else {
            msg.into()
        };
        assert!(
            err.contains(&want) && err.contains(note),
            "{top} {mode:?}: {err}"
        );
        shown.push(err[err.find("error[").expect("an error")..].to_string());
        if !mode.is_empty() {
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{err}"
            );
        }
    }
    assert_eq!(shown[0], shown[2], "{top}");
    assert_eq!(bare(&shown[0]), shown[1], "{top}");
}

#[test]
fn failed_checks_in_region_code_stop_the_run_as_in_the_normal_mode() {
    let dir = tempdir("cycle-checks");
    analyze(&dir, "checks.vhd", CHECKS);
    for (top, msg, note) in [
        (
            "cycle_div",
            "error[E3009]: division by zero",
            "at 30 ns, delta 1, in process cycle_div.reg",
        ),
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
        same_failure(&dir, top, msg, note);
    }
    // With the compilation cache: code compiled without the values is never reused under
    // the metavalue contract, nor the other way round.
    let msg = "error[E3009]: result of 1162261467 * 3 cannot be represented as INTEGER";
    for exact in [false, true, false, true] {
        let mut args = vec!["run", "--work=work:lib", "--tier=interp", "--cycle"];
        args.extend((!exact).then_some("--two-state"));
        args.push("cycle_ovf");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{exact}: {err}");
        let want = if exact { msg.into() } else { bare(msg) };
        assert!(err.contains(&want), "{exact}: {err}");
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
        same_failure(&dir, top, msg, note);
    }
}

const SIG_CHECKS: &str = "\
entity cycle_sigact is
end entity;
architecture a of cycle_sigact is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer range 0 to 9 := 0;
  procedure drv(signal o : out integer; v : integer) is
  begin
    o <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, n + 5);
    end if;
  end process;
end architecture;

entity cycle_sigfml is
end entity;
architecture a of cycle_sigfml is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer := 0;
  procedure drv(signal o : out integer range 0 to 5; v : integer) is
  begin
    o <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, n);
    end if;
  end process;
end architecture;

entity cycle_sigfwd is
end entity;
architecture a of cycle_sigfwd is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer := 0;
  procedure p3(signal p : out integer; v : integer) is
  begin
    p <= v;
  end procedure;
  procedure p2(signal o : inout integer; v : integer) is
  begin
    p3(o, v);
  end procedure;
  procedure p1(signal o2 : inout integer range 0 to 30; v : integer) is
  begin
    p2(o2, v);
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      p1(s, n * 5);
    end if;
  end process;
end architecture;

entity cycle_sigelem is
end entity;
architecture a of cycle_sigelem is
  signal clk : bit := '0';
  signal n : integer := 0;
  type arr is array (0 to 3) of integer range 0 to 9;
  signal s : arr := (others => 0);
  procedure drv(signal o : out integer; v : integer) is
  begin
    o <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s(2), n + 5);
    end if;
  end process;
end architecture;

entity cycle_sigfield is
end entity;
architecture a of cycle_sigfield is
  signal clk : bit := '0';
  signal n : integer := 0;
  type rec is record
    a : integer range 0 to 9;
    b : integer;
  end record;
  signal s : rec := (0, 0);
  procedure drv(signal o : out integer; v : integer) is
  begin
    o <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s.a, n + 5);
    end if;
  end process;
end architecture;

entity cycle_sigcond is
end entity;
architecture a of cycle_sigcond is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer range 0 to 9 := 0;
  procedure drv(signal o : out integer; v : integer) is
  begin
    o <= v when v > 2 else 1;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, n + 5);
    end if;
  end process;
end architecture;

package cycle_sigpk is
  procedure drv(signal o : out integer; v : integer);
end package;
package body cycle_sigpk is
  procedure drv(signal o : out integer; v : integer) is
  begin
    o <= v;
  end procedure;
end package body;

use work.cycle_sigpk.all;
entity cycle_sigpkg is
end entity;
architecture a of cycle_sigpkg is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer range 0 to 9 := 0;

begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, n + 5);
    end if;
  end process;
end architecture;

entity cycle_sigin is
end entity;
architecture a of cycle_sigin is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal o : integer := 0;
  procedure rd(signal v : in integer range 0 to 5; signal q : out integer) is
  begin
    q <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      rd(n, o);
      report \"o=\" & integer'image(o);
    end if;
  end process;
end architecture;

entity cycle_signest is
end entity;
architecture a of cycle_signest is
  signal clk : bit := '0';
  signal n : integer := 0;
  signal s : integer range 0 to 9 := 0;
  procedure drv(signal o : out integer; v : integer) is
    function same(x : integer) return integer is
    begin
      return x;
    end function;
  begin
    o <= same(v);
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, n + 5);
      report \"s=\" & integer'image(s);
    end if;
  end process;
end architecture;

";

/// A value driven through a scalar signal formal is checked against the formal's subtype, then
/// against its actual's (LRM 14.7.3, #1186), with the normal mode's messages: through a formal
/// passed on twice, an element or field actual, a conditional waveform and a package procedure.
/// Nothing is checked at the call: an `in` formal narrower than its actual's value is not an
/// error in either mode. A procedure that declares a subprogram is not checked against its
/// actual in either mode yet (the normal mode makes no checked body for it).
#[test]
fn values_driven_through_signal_formals_are_checked_as_in_the_normal_mode() {
    let dir = tempdir("cycle-sig-checks");
    analyze(&dir, "checks.vhd", SIG_CHECKS);
    let act =
        "error[E3009]: value 10 outside of INTEGER range 0 to 9 for actual of signal parameter O";
    for (top, msg, note) in [
        (
            "cycle_sigact",
            act,
            "at 55 ns, delta 0, in process cycle_sigact.reg",
        ),
        (
            "cycle_sigfml",
            "error[E3009]: value 6 outside of INTEGER range 0 to 5 for parameter O",
            "at 65 ns, delta 0, in process cycle_sigfml.reg",
        ),
        (
            "cycle_sigfwd",
            "error[E3009]: value 35 outside of INTEGER range 0 to 30 for actual of signal parameter P",
            "at 75 ns, delta 0, in process cycle_sigfwd.reg",
        ),
        (
            "cycle_sigelem",
            act,
            "at 55 ns, delta 0, in process cycle_sigelem.reg",
        ),
        (
            "cycle_sigfield",
            act,
            "at 55 ns, delta 0, in process cycle_sigfield.reg",
        ),
        (
            "cycle_sigcond",
            act,
            "at 55 ns, delta 0, in process cycle_sigcond.reg",
        ),
        (
            "cycle_sigpkg",
            act,
            "at 55 ns, delta 0, in process cycle_sigpkg.reg",
        ),
    ] {
        same_failure(&dir, top, msg, note);
    }
    let base = run(&dir, &[], "cycle_sigin");
    for mode in &MODES[1..] {
        let (out, err) = run(&dir, mode, "cycle_sigin");
        assert_eq!(out, base.0, "{mode:?}");
        assert!(
            err.contains("cycle mode: 1 of 2 processes run cycle-based"),
            "{err}"
        );
    }
    let base = run(&dir, &[], "cycle_signest");
    for mode in &MODES[1..] {
        let (out, err) = run(&dir, mode, "cycle_signest");
        assert_eq!(out, base.0, "{mode:?}");
        assert!(
            err.contains("cycle mode: 1 of 2 processes run cycle-based"),
            "{err}"
        );
    }
}

const MSG_CHECKS: &str = "\
entity cycle_menum is
end entity;
architecture a of cycle_menum is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable v : rgb;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      v := color'val(n mod 4);
      r <= color'pos(v);
    end if;
  end process;
end architecture;

entity cycle_mrev is
end entity;
architecture a of cycle_mrev is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable v : rev;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      v := color'val(3 - n mod 4);
      r <= color'pos(v);
    end if;
  end process;
end architecture;

entity cycle_msig is
end entity;
architecture a of cycle_msig is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
  signal s : rgb := red;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      s <= color'val(n mod 4);
    end if;
  end process;
end architecture;

entity cycle_mval is
end entity;
architecture a of cycle_mval is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable v : color;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      v := color'val(n);
      r <= color'pos(v);
    end if;
  end process;
end architecture;

entity cycle_mchar is
end entity;
architecture a of cycle_mchar is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable c : lc;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      c := character'val(97 + n);
      r <= character'pos(c);
    end if;
  end process;
end architecture;

entity cycle_mindex is
end entity;
architecture a of cycle_mindex is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable t : tab := (others => 0);
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      t(color'val(n mod 4)) := n;
      r <= t(red);
    end if;
  end process;
end architecture;

entity cycle_mcindex is
end entity;
architecture a of cycle_mcindex is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable t : ctab := (others => 0);
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      t(character'val(97 + n)) := n;
      r <= t('a');
    end if;
  end process;
end architecture;

entity cycle_mnested is
end entity;
architecture a of cycle_mnested is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  type vtab is array (rgb) of bit_vector(3 downto 0);
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : bit_vector(3 downto 0);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable m : vtab;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      m(color'val(n mod 4)) := \"0101\";
      r <= m(red);
    end if;
  end process;
end architecture;

entity cycle_mformal is
end entity;
architecture a of cycle_mformal is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
  signal s : rgb := red;
  procedure drv(signal o : out color; v : color) is
  begin
    o <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      drv(s, color'val(n mod 4));
    end if;
  end process;
end architecture;

entity cycle_mphys is
end entity;
architecture a of cycle_mphys is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  subtype rev is color range blue downto green;
  subtype lc is character range 'a' to 'c';
  subtype short is time range 1 ns to 10 ns;
  type tab is array (rgb) of integer;
  type ctab is array (lc) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : integer := 0;
begin
  clk <= not clk after 5 ns when now < 200 ns;
  reg : process (clk)
    variable t : short := 1 ns;
  begin
    if clk'event and clk = '1' then
      n <= n + 1;
      t := (n + 1) * 1 ns;
      r <= t / 1 ns;
    end if;
  end process;
end architecture;

";

/// A failed range or index check of an enumeration, a character or a physical value shows its
/// values as the normal mode does (#1188): literals (a descending subtype `downto`, an index of
/// an array of vectors) and units;
/// `T'val` outside its type keeps positions in both modes. A physical value keeps its process
/// in the event kernel.
#[test]
fn range_and_index_messages_show_literals_as_in_the_normal_mode() {
    let dir = tempdir("cycle-msg-checks");
    analyze(&dir, "checks.vhd", MSG_CHECKS);
    for (top, msg, at) in [
        (
            "cycle_menum",
            "value WHITE outside of RGB range RED to BLUE for variable V",
            35,
        ),
        (
            "cycle_mrev",
            "value WHITE outside of REV range BLUE downto GREEN for variable V",
            5,
        ),
        (
            "cycle_msig",
            "value WHITE outside of RGB range RED to BLUE for signal S",
            35,
        ),
        ("cycle_mval", "value 4 outside of COLOR range 0 to 3", 45),
        (
            "cycle_mchar",
            "value 'd' outside of LC range 'a' to 'c' for variable C",
            35,
        ),
        (
            "cycle_mindex",
            "index WHITE outside of range RED to BLUE for t",
            35,
        ),
        (
            "cycle_mcindex",
            "index 'd' outside of range 'a' to 'c' for t",
            35,
        ),
        (
            "cycle_mnested",
            "index WHITE outside of range RED to BLUE for m",
            35,
        ),
        (
            "cycle_mformal",
            "value WHITE outside of COLOR range RED to BLUE for actual of signal parameter O",
            35,
        ),
    ] {
        let note = format!("at {at} ns, delta 0, in process {top}.reg");
        same_failure(&dir, top, &format!("error[E3009]: {msg}"), &note);
    }
    let msg = "error[E3009]: value 11 NS outside of SHORT range 1 NS to 10 NS for variable T";
    for mode in MODES {
        let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
        args.extend(mode);
        args.push("cycle_mphys");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{mode:?}: {err}");
        assert!(err.contains(msg), "{mode:?}: {err}");
    }
    // A failed check of code from the compilation cache: its site keeps the type too.
    let msg = "error[E3009]: value WHITE outside of RGB range RED to BLUE for variable V";
    for exact in [false, true, false, true] {
        let mut args = vec!["run", "--work=work:lib", "--tier=interp", "--cycle"];
        args.extend((!exact).then_some("--two-state"));
        args.push("cycle_menum");
        let o = yavs(&dir, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{exact}: {err}");
        let want = if exact { msg.into() } else { bare(msg) };
        assert!(err.contains(&want), "{exact}: {err}");
    }
}

/// A call whose constant actual lies outside its formal's subtype stays in the event kernel,
/// which fails the check as the normal mode does.
#[test]
fn a_constant_actual_outside_its_formal_keeps_the_call_out() {
    let dir = tempdir("cycle-call-const");
    analyze(&dir, "checks.vhd", CALL_CHECKS);
    for mode in [&[][..], &["--cycle", "--two-state"]] {
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
        if !mode.is_empty() {
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
    let (cycle, err) = run(
        &dir,
        &["--tier=interp", "--cycle", "--two-state"],
        "cycle_fz",
    );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_ops2",
            );
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
        let flags = ["--tier=interp", "--cycle", "--two-state"];
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_calls",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_recarr",
            );
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
            // The default reads every metavalue as 0 and keeps nothing out for one (A31).
            let (two, terr) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_zpass",
            );
            assert!(
                terr.contains("cycle mode: 8 of 20 processes run cycle-based"),
                "{w}: {terr}"
            );
            let zeros: Vec<String> = (k(normal.clone()).iter())
                .map(|l| l.replace(['Z', 'X'], "0"))
                .collect();
            assert_eq!(k(two), zeros, "width {w} {t:?}\n{terr}");
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

/// Clocks copied by assignment are their source (A32): an aggregate of the clock, a port
/// connection and a plain copy give one edge class with it, so the logic between the source's
/// registers and the copies' joins and is sampled before it settles, as in the normal mode; a
/// register of the source read directly on a copy gives its value before the edge (the normal
/// mode, a delta later, the new one: the documented delta-order loss). Copies inside an entity
/// of its clock port chosen by a generic, a falling edge of a copy starting at `'U'`, and two
/// copies of a clock nothing else tests (one class of the two) are copies too. An inverted and a
/// gated clock, a copy of a clock a register of the region makes, and a copy whose start value
/// gives it an edge at time 0 its source does not have keep classes of their own, and so do the
/// members on a copy that a register of the event kernel on that copy samples.
/// `YAVS_CYCLE_EXACT_CLOCKS=1` keeps every copy a clock of its own: then the logic stays out and
/// every value is the normal mode's.
/// `YAVS_CYCLE_DOWNSTREAM` lists the processes outside that read what the region exports, directly
/// and through processes outside and ports, and no other: the gates' rule for library warnings of
/// processes outside (tools/difftest README "Contract classes") rests on it.
#[test]
fn the_processes_downstream_of_the_region_are_listed() {
    for w in [1, 63, 64, 65, 128] {
        let dir = tempdir(&format!("cycle-down-{w}"));
        let text = include_str!("cycle/downstream.vhd").replace("W - 1", &format!("{} - 1", w));
        analyze(&dir, "downstream.vhd", &text);
        let file = dir.join("down.txt");
        let f = file.to_string_lossy().into_owned();
        let o = yavs_env(
            &dir,
            &[
                "run",
                "--work=work:lib",
                "--no-cache",
                "--cycle-report",
                "--two-state",
                "cycle_down",
            ],
            &[("YAVS_CYCLE_DOWNSTREAM", &f)],
        );
        let report = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{report}");
        let member = |n: &str| {
            (report.lines()).any(|l| l.trim_start().starts_with(n) && l.ends_with("clocked"))
        };
        assert!(member("cycle_down.dut.reg (downstream.vhd:11)"), "{report}");
        assert!(report.contains("cycle mode: 1 of "), "{report}");
        let down = std::fs::read_to_string(&file).expect("the file");
        let names: Vec<&str> = down.lines().collect();
        assert!(
            names.iter().any(|n| n.starts_with("cycle_down.reader ")),
            "{down}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("cycle_down.mon.chain ")),
            "{down}"
        );
        assert_eq!(names.len(), 2, "{down}\n{report}");
    }
}

#[test]
fn clocks_copied_by_assignment_are_their_source() {
    let dir = tempdir("cycle-clkcopy");
    analyze(&dir, "clkcopy.vhd", include_str!("cycle/clkcopy.vhd"));
    let no_c = |v: &[String]| -> Vec<String> {
        (v.iter())
            .map(|l| {
                let (a, b) = (l.split(" c=").next(), l.split(" d=").nth(1));
                format!("{} d={}", a.unwrap_or(""), b.unwrap_or(""))
            })
            .collect()
    };
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_clkcopy");
        assert_eq!(normal.len(), 13, "{normal:?}");
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_clkcopy",
        );
        assert!(
            err.contains("cycle mode: 21 of 36 processes run cycle-based"),
            "{err}"
        );
        let edges = err.lines().find(|l| l.contains(", on the ")).unwrap_or("");
        for copy in ["element 1", "element 3", ".one", "dist.cl", ".b2"] {
            assert!(!edges.contains(copy), "{copy}: {edges}");
        }
        for own in [
            ".inv",
            ".gated",
            "(element 2)",
            ".hcopy",
            ".dcopy",
            ".b1",
            ".two",
            "falling edge of cycle_clkcopy.clk",
        ] {
            assert!(edges.contains(own), "{own}: {edges}");
        }
        assert_eq!(no_c(&cycle), no_c(&normal), "{t:?}\n{err}");
        // `c` samples `a` before the edge: three less than the normal mode's, after the first.
        assert!(
            cycle[2].contains(" c=3 ") && normal[2].contains(" c=6 "),
            "{cycle:?}"
        );
        let exact = [("YAVS_CYCLE_EXACT_CLOCKS", "1")];
        let (cycle, err) = run_vars(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_clkcopy",
            &exact,
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 18 of 36 processes run cycle-based"),
            "{err}"
        );
        for out in [
            "cycle_clkcopy.m <= ...",
            "dist.s <= ...",
            "cycle_clkcopy.v <= ...",
        ] {
            assert!(err.contains(out), "{out}: {err}");
        }
    }
    let o = yavs_env(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle",
            "--two-state",
            "cycle_clkcopy",
        ],
        &[("YAVS_CYCLE_EXACT_CLOCKS", "yes")],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        !o.status.success() && err.contains("E4104") && err.contains("takes `1`"),
        "{err}"
    );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_recarrrec",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_logicconv",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_slices",
            );
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
    let (cycle, err) = run(
        &dir,
        &["--tier=interp", "--cycle", "--two-state"],
        "cycle_d14",
    );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_reclit",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_intsig",
            );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_consts",
        );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_constelem",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_selfassign",
            );
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 3 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// Procedure calls with mode views (VHDL-2019): a view reads its `in` and `inout` elements and
/// drives its `out` ones. A concurrent call driving only through a view had no outputs in the
/// region, so the logic reading them was a wake late; calls through a nested view, a converse
/// view and an alias join with the normal result. A procedure that may return before it assigns
/// its `out` is a latch, not a member.
#[test]
fn procedure_calls_with_mode_views_read_and_drive_their_elements() {
    let dir = tempdir("cycle-views");
    std::fs::write(dir.join("views.vhd"), include_str!("cycle/views.vhd")).expect("write design");
    let o = yavs(
        &dir,
        &["analyze", "--std=2019", "--work=work:lib", "views.vhd"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_views");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_views",
            );
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 7 of 11 processes run cycle-based"),
                "{w}: {err}"
            );
            for p in ["outs", "c1", "c2", "clocked", "comb"] {
                let member = err.lines().any(|l| {
                    l.trim_start().starts_with(&format!("cycle_views.{p} ("))
                        && (l.ends_with("clocked") || l.ends_with("combinational"))
                });
                assert!(member, "{p}: {err}");
            }
            assert!(err.contains("E4119 a latch: cycle_views.lt"), "{err}");
        }
    }
}

/// An asynchronous reset with a constant in an `and` (`rst = '1' and G`, `G and rst = '1'`) is the
/// reset alone for G true and no reset for G false: the registers join, also in the normal mode's
/// edge trigger. A reset of another form and an assert before the edge test keep their process out
/// with a reason that names them, not "a call of `rising_edge`".
#[test]
fn a_reset_with_a_constant_in_an_and_folds() {
    let dir = tempdir("cycle-foldreset");
    analyze(&dir, "foldreset.vhd", include_str!("cycle/foldreset.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_foldreset");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_foldreset",
            );
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            // The reset pulse at 60 ns: G true resets, G false keeps the data (hand-checked).
            if w == 1 {
                assert!(normal[5].ends_with("q=0 1 0 1 1 0 0 1"), "{normal:?}");
            }
            assert!(
                err.contains("cycle mode: 4 of 10 processes run cycle-based"),
                "{w}: {err}"
            );
            for p in ["on_g.first", "on_g.second", "off.first", "off.second"] {
                let member = err.lines().any(|l| {
                    l.trim_start()
                        .starts_with(&format!("cycle_foldreset.{p} ("))
                        && l.ends_with("clocked")
                });
                assert!(member, "{p}: {err}");
            }
            assert!(
                err.contains(
                    "E4101 an asynchronous reset condition of this form: cycle mode takes one \
                     signal of a logic type compared with a value (`rst = '1'`), alone or in an \
                     `and` with constants (foldreset.vhd:32)"
                ),
                "{err}"
            );
            assert!(
                err.contains(
                    "E4101 a clocked process with statements beside its edge test: cycle mode \
                     takes `if <edge> then ... end if;`, after an optional reset branch, as its \
                     only statement (foldreset.vhd:38)"
                ),
                "{err}"
            );
        }
    }
}

/// A recursion elaboration does not bound is inlined to the cap (`DEPTH_MAX`) and stays out, on an
/// 8 MiB stack, the usual size of the main thread: an unoptimised build overflowed it at a cap
/// of 32 (its frames are about ten times larger; it caps at 12).
#[cfg(unix)]
#[test]
fn a_recursion_to_the_inlining_cap_fits_an_8_mib_stack() {
    let dir = tempdir("cycle-depth");
    analyze(
        &dir,
        "depth.vhd",
        "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\n\
         entity cycle_depth is end entity;\narchitecture a of cycle_depth is\n\
         \x20 function fact (n : natural) return natural is\n  begin\n    if n = 0 then\n\
         \x20     return 1;\n    end if;\n    return (n * fact(n - 1)) mod 256;\n  end function;\n\
         \x20 signal clk : std_ulogic := '0';\n  signal ad : unsigned(2 downto 0) := \"000\";\n\
         \x20 signal o : std_ulogic_vector(7 downto 0);\nbegin\n\
         \x20 p : process (clk)\n  begin\n    if rising_edge(clk) then\n\
         \x20     o <= std_ulogic_vector(to_unsigned(fact(to_integer(ad)), 8));\n\
         \x20   end if;\n  end process;\nend architecture;\n",
    );
    let o = Command::new("sh")
        .current_dir(&dir)
        .env("YAVS_STD_CACHE", "off")
        .args(["-c", "ulimit -s 8192 && exec \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_yavs"))
        .args([
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-report",
            "--two-state",
            "cycle_depth",
        ])
        .output()
        .expect("run yavs");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{:?}: {err}", o.status);
    assert!(
        err.contains("E4101 calls of `fact` nested more than "),
        "{err}"
    );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_shiftfill",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_joinif",
            );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_constcall",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_intelem1",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_runreset",
        );
        assert_eq!(normal.len(), 12, "{t:?} {normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 7 processes run cycle-based")
                && err.contains(
                    "a register with an asynchronous reset feeding a process that depends on delta \
                     cycles: it drives cycle_runreset.q, which cycle_runreset.dut.list reads"
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_pulse",
        );
        assert_eq!(normal.len(), 12, "{t:?} {normal:?}");
        assert!(normal[2].contains("t=00"), "{normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 6 processes run cycle-based")
                && err.contains("its asynchronous reset cycle_pulse.r comes from a register"),
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_intarr",
        );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_ram",
            );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_shapes",
        );
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
            let (cycle, err) = run_text(
                &dir,
                &[&flags[..], &["--cycle", "--two-state"]].concat(),
                top,
            );
            assert!(normal.contains("Note"), "{name}: {normal}");
            assert_eq!(cycle, normal, "{name} {t:?}\n{err}");
            assert!(
                err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                "{name}: {err}"
            );
        }
    }
}

/// Bench designs whose processes cannot join (yet), with the reason the report gives; `rtl_lfsr`'s
/// processes are of the top-level unit, whose compares with a metavalue literal keep them out (A57).
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
        let (cycle, err) = run_text(
            &dir,
            &[&flags[..], &["--cycle", "--two-state"]].concat(),
            top,
        );
        assert_eq!(cycle, normal, "{name}\n{err}");
        assert!(err.contains(" 0 of "), "{name}: {err}");
        assert!(err.contains(why), "{name}: {err}");
    }
}

/// `std_match` and `?=` with a `'-'` aggregate whose length a generic sets: it matches anything in
/// the region too (it was compared with 0 there, a wrong result, #1192); an aggregate of a `'-'`
/// and other values keeps a matching comparison out, under both contracts.
#[test]
fn a_dont_care_aggregate_of_a_generic_length_matches_anything() {
    let dir = tempdir("cycle-matchagg");
    analyze(&dir, "matchagg.vhd", include_str!("cycle/matchagg.vhd"));
    let (normal, _) = run(&dir, &["--tier=interp"], "matchagg");
    assert_eq!(normal.len(), 15, "{normal:?}");
    assert!(
        normal.iter().all(|l| l.contains("m='1' q='1'")),
        "{normal:?}"
    );
    for t in tiers() {
        for exact in [false, true] {
            let mut f = t.clone();
            f.push("--cycle");
            if !exact {
                f.push("--two-state");
            }
            let (cycle, err) = run(&dir, &f, "matchagg");
            assert_eq!(cycle, normal, "{f:?}: {err}");
            // The first member of each instance joins, the one with the mixed aggregate stays out.
            assert!(err.contains("cycle mode: 5 of"), "{f:?}: {err}");
        }
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
            "--two-state",
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
                // In the subset, kept out by the lowering or a rule of the region as a whole.
                h if h
                    .get(1..5)
                    .and_then(|n| n.parse::<u32>().ok())
                    .is_some_and(|n| n == 4101 || n == 4140 || (4128..=4137).contains(&n)) =>
                {
                    "in the subset, not implemented in cycle mode yet:"
                }
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
        ("external", "E4138", "<<signal"),
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
        ("slow_loop", "E4139", "while"),
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
            r.contains("E4119 a latch: sub_tb.bus16(7) is not assigned on every path")
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
        assert!(r.contains("  E4122 a combinational loop: "), "{r}");
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
        ("waits_inside", "it calls `tick`, which can wait"),
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
    // Asserts and reports are evaluated: nothing is listed as not evaluated.
    assert!(!report.contains("not evaluated in cycle mode"), "{report}");
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

/// A force keeps out only the drivers of what it can force (`tests/cycle/forcescope.vhd`): a
/// force in a unit the design does not instantiate or a package procedure nobody calls keeps
/// nothing out; one through an external name in a process or a package procedure keeps the
/// forced register out while members read the forced value (forced and released in the delta of
/// an edge and between edges); a forced clock copy keeps its register on the copy, and a forced
/// clock its own delta class; a force of a signal parameter, and any force under
/// the metavalue contract (no `--two-state`), keeps every process out, but one the design does not reach does not
/// stop ports from collapsing (`fsc_unreached_tb`). Against the
/// normal mode at widths 1 to 128.
#[test]
fn forces_keep_out_only_the_drivers_of_what_they_force() {
    let dir = tempdir("cycle-force-scope");
    let fdir = tempdir("cycle-force-formal");
    for w in [1, 63, 64, 65, 128] {
        let src = with_width(include_str!("cycle/forcescope.vhd"), w);
        analyze(&dir, &format!("forcescope{w}.vhd"), &src);
        analyze(&fdir, &format!("forcescope{w}.vhd"), &src);
        analyze(
            &fdir,
            &format!("forcescopef{w}.vhd"),
            &with_width(include_str!("cycle/forcescopef.vhd"), w),
        );
        // `fsc_unreached_tb` uses a package whose procedure forces a signal parameter but never
        // calls it: its ports collapse and the same processes join as in `fsc_ext_tb`.
        let cases: [(&Path, &str, &[&str], usize, &str); 5] = [
            (&dir, "fsc_ext_tb", &[], 16, "5 of 9"),
            (&dir, "fsc_pkg_tb", &[], 16, "5 of 9"),
            (&dir, "fsc_dclk_tb", &["--stop-time=40ns"], 6, "2 of 7"),
            (&fdir, "fsc_formal_tb", &[], 16, "0 of 9"),
            (&fdir, "fsc_unreached_tb", &[], 16, "5 of 9"),
        ];
        for (d, top, flags, n, members) in cases {
            for t in tiers() {
                let (normal, _) = run(d, &[&t[..], flags].concat(), top);
                let (cycle, err) = run(
                    d,
                    &[&t[..], flags, &["--cycle", "--two-state"]].concat(),
                    top,
                );
                assert_eq!(normal.len(), n, "{w} {top} {t:?} {normal:?}");
                assert_eq!(cycle, normal, "width {w} {top} {t:?}\n{err}");
                assert!(
                    err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                    "{w} {top}: {err}"
                );
                // The metavalue contract does not follow forced values: every process stays out.
                if top == "fsc_ext_tb" {
                    let ex = [&t[..], &["--cycle"]].concat();
                    let (exact, err) = run(d, &ex, top);
                    assert_eq!(exact, normal, "width {w} {top} {t:?}\n{err}");
                    assert!(err.contains("cycle mode: 0 of 9 processes"), "{err}");
                    assert!(
                        err.contains("without `--two-state` no process joins"),
                        "{err}"
                    );
                }
            }
        }
    }
    for top in ["fsc_ext_tb", "fsc_pkg_tb"] {
        let report = verdicts(&dir, top);
        let (_, r) = row(&report, &format!("{top}.dut.p_mid"));
        assert!(
            r.contains(&format!(
                "E4128 a signal forced elsewhere in the design: {top}.dut.mid"
            )),
            "{r}"
        );
        for p in ["p_q", "p_r", "mid2 <= ...", "mo <= ...", "m2o <= ..."] {
            let (s, r) = row(&report, &format!("{top}.dut.{p}"));
            assert_eq!(s, "cycle-based:", "{r}");
        }
    }
    let report = verdicts(&dir, "fsc_dclk_tb");
    let (_, r) = row(&report, "fsc_dclk_tb.mid <= ...");
    assert!(r.contains("E4130 "), "{r}");
    let report = verdicts(&fdir, "fsc_formal_tb");
    let (_, r) = row(&report, "fsc_formal_tb.dut.p_q");
    assert!(
        r.contains("E4128 `force` or `release` in the design"),
        "{r}"
    );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_null",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_wrong",
        );
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
            "--two-state",
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
            err.contains(&format!("error[{code}]: "))
                && err.contains(&format!(
                    "= note: in process sub_tb.dut.{label} (subset.vhd:"
                )),
            "{code} {label}: {err}"
        );
    }
    // In the subset but not implemented yet: the code of that.
    let at = err
        .find("note: in process sub_tb.dut.async_two")
        .expect("listed");
    assert!(
        err[..at]
            .rfind("error[")
            .is_some_and(|i| err[i..].starts_with("error[E4101]")),
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_ports",
        );
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
            let (cycle, err) = run(
                &dir,
                &[&flags[..], &["--cycle", "--two-state"]].concat(),
                "cycle_reset",
            );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_clocks",
        );
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
        assert!(
            line.contains("with its edge in a later delta cycle"),
            "{line}"
        );
    }
}

#[test]
fn asynchronous_resets_made_inside_the_region_give_the_same_result() {
    let dir = tempdir("cycle-inreset");
    analyze(&dir, "inreset.vhd", include_str!("cycle/inreset.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_inreset");
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_inreset",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_history",
        );
        assert_eq!(normal.len(), 72, "{normal:?}");
        // With the drivers in the region the redrives of `r` vanish (`'transaction`, `'active`).
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("cycle mode: 2 of 8 processes run cycle-based"),
            "{err}"
        );
        assert_eq!(row(&err, "cycle_history.dut.pn").0, "cycle-based:", "{err}");
        for (label, what) in [
            (
                "pq",
                "`'last_event` of cycle_history.q, read by cycle_history.main",
            ),
            (
                "pr",
                "`'active` of cycle_history.r, read by cycle_history.main",
            ),
            ("pt", "an implicit signal"),
            (
                "pk",
                "a call of a subprogram reading the history of cycle_history.k",
            ),
        ] {
            let (section, line) = row(&err, &format!("cycle_history.dut.{label}"));
            assert_eq!(section, "outside the subset, in the event kernel:", "{err}");
            assert!(
                line.contains("E4127 history of a signal read outside the region: ")
                    && line.contains(what),
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
            "--two-state",
            "cycle_history",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    for label in ["pq", "pr", "pt", "pk"] {
        assert!(
            err.contains(&format!(
                "= note: in process cycle_history.dut.{label} (history.vhd:"
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_inputs",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_tbl",
        );
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_loops",
        );
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
    for mode in [&[][..], &["--cycle", "--two-state"]] {
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
        let msg = "error[E3009]: value 8 outside of INTEGER range 0 to 7 for signal K2";
        let want = if !mode.is_empty() {
            bare(msg)
        } else {
            msg.into()
        };
        assert!(
            err.contains(&want) && err.contains("in process cycle_loops.pg"),
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_early",
        );
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
        let (out, err, rc) = run_env(
            &dir,
            &env,
            &["--tier=interp", "--cycle", "--two-state"],
            "cycle_cut",
        );
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
            &["--tier=interp", "--cycle", "--two-state", "-gLIMIT=5000"],
            "cycle_cut",
        );
        assert_eq!(rc, 1, "{chunk:?}: {err}");
        for l in &failure {
            assert!(err.contains(&bare(l)), "{chunk:?}: {l}: {err}");
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
            .args(["--cycle", "--two-state", "-gLIMIT=5000", "cycle_cut"])
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
            assert!(err.contains(&bare(l)), "{pass}: {l}: {err}");
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
        let (out, err, rc) = run_env(
            &dir,
            &env,
            &["--tier=interp", "--cycle", "--two-state"],
            "cycle_share",
        );
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
            &["--tier=interp", "--cycle", "--two-state", "-gFAIL=true"],
            "cycle_share",
        );
        assert_eq!(rc, 1, "{chunk:?}: {err}");
        for l in &failure {
            assert!(err.contains(&bare(l)), "{chunk:?}: {l}: {err}");
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
                let flags = [&t[..], &[mode, "--two-state"]].concat();
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
        let report = with_report(
            &dir,
            &["--cycle", "--two-state"],
            "cycle_topcheck",
            String::new(),
        );
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
                .contains("E4136 an assert reading a signal the region does not drive alone: cycle_topcheck.flag"),
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
    let (want, n, _) = cached(&dir, &one, &["--cycle", "--two-state", "--no-cache"]);
    assert!(n >= 4, "{n} functions");
    let four = [("YAVS_CYCLE_CHUNK", "5"), ("YAVS_CYCLE_THREADS", "4")];
    let (got, n4, hits) = cached(&dir, &four, &["--cycle", "--two-state"]);
    assert_eq!((&got, n4, hits), (&want, n, 0), "four threads, cold");
    let check = [
        ("YAVS_CYCLE_CHUNK", "5"),
        ("YAVS_CYCLE_THREADS", "1"),
        ("YAVS_CYCLE_CACHE_CHECK", "1"),
    ];
    let (got, _, hits) = cached(&dir, &check, &["--cycle", "--two-state"]);
    assert_eq!(
        (&got, hits),
        (&want, 0),
        "one thread, every function checked"
    );
    std::fs::remove_dir_all(&obj).expect("empty the cache");
    let chunk = [("YAVS_CYCLE_CHUNK", "5")];
    let (got, _, hits) = cached(&dir, &chunk, &["--cycle", "--two-state", "--threads=4"]);
    assert_eq!((&got, hits), (&want, 0), "--threads=4, cold");
    let (got, _, hits) = cached(&dir, &chunk, &["--cycle", "--two-state", "--threads=4"]);
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

    let no_cache = cached(&dir, &[], &["--cycle", "--two-state", "--no-cache"]);
    assert_eq!((no_cache.0.as_str(), no_cache.2), (want.as_str(), 0));
    assert!(entries().is_empty(), "--no-cache wrote {:?}", entries());

    let (cold, n, hits) = cached(&dir, &[], &["--cycle", "--two-state"]);
    assert_eq!((cold.as_str(), hits), (want.as_str(), 0));
    assert!(n > 0 && !entries().is_empty());
    let (warm, n2, hits) = cached(&dir, &[], &["--cycle", "--two-state"]);
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
        (
            "generic",
            &[][..],
            &["--cycle", "--two-state", "-gW=12"][..],
            &w12,
        ),
        (
            "checks",
            &checks[..],
            &["--cycle", "--two-state"][..],
            &want,
        ),
        ("build", &salt[..], &["--cycle", "--two-state"][..], &want),
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
    let (got, _, hits) = cached(&dir, &regs, &["--cycle", "--two-state"]);
    assert_eq!((&got, hits), (&want, 0), "two words in registers");
    analyze(
        &dir,
        "cache.vhd",
        &include_str!("cycle/cache.vhd").replace("STEP : natural := 3", "STEP : natural := 5"),
    );
    let step5 = normal(&["--tier=interp"]);
    assert_ne!(step5, want);
    let (got, n, hits) = cached(&dir, &[], &["--cycle", "--two-state"]);
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
        let (got, n, hits) = cached(&dir, &[], &["--cycle", "--two-state"]);
        assert_eq!((&got, hits), (&step5, 0), "damage {damage}");
        let (got, n2, hits) = cached(&dir, &[], &["--cycle", "--two-state"]);
        assert_eq!((&got, n2, hits), (&step5, n, n), "damage {damage}, warm");
    }

    // Two runs at once on an empty cache.
    for p in entries() {
        std::fs::remove_file(p).expect("remove entry");
    }
    let racers: Vec<_> = (0..2)
        .map(|_| {
            let d = dir.to_path_buf();
            std::thread::spawn(move || cached(&d, &[], &["--cycle", "--two-state", "-gW=9"]))
        })
        .collect();
    let w9 = normal(&["--tier=interp", "-gW=9"]);
    for r in racers {
        assert_eq!(r.join().expect("racer").0, w9);
    }
    let (got, n, hits) = cached(&dir, &[], &["--cycle", "--two-state", "-gW=9"]);
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

    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state", "--no-cache"]);
    assert_eq!((got.as_str(), a.as_str()), (want.as_str(), ""));
    assert!(entries().is_empty(), "--no-cache wrote {:?}", entries());

    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(
        a.contains(stored)
            && a.contains("generics W (walk), L (walk), F (value passes), K (selection)"),
        "{a}"
    );
    assert!(!a.contains("TAG"), "{a}");
    let (got, a, f) = acached(&dir, &[], &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(a.contains(hit), "a warm run: {a}");
    assert!(a.contains("region from the cache"), "a warm run: {a}");
    assert!(f.contains("from the cache"), "{f}");
    let check = [("YAVS_CYCLE_CACHE_CHECK", "1")];
    let (got, a, _) = acached(&dir, &check, &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(
        a.contains("analysis from the cache checked: the same"),
        "{a}"
    );
    assert!(a.contains("region from the cache checked: the same"), "{a}");
    // `--cycle-strict` builds the region from the selection in the cache, with its code.
    let (got, a, f) = acached(&dir, &[], &["--cycle-strict", "--two-state"]);
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
    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(
        a.contains(hit) && !a.contains("region from"),
        "damaged region: {a}"
    );
    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(a.contains("region from the cache"), "rebuilt region: {a}");

    // Read by nothing before the run: the same analysis.
    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state", "-gTAG=y"]);
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
    let (got, a, _) = acached(&dir, &other, &["--cycle", "--two-state"]);
    assert_eq!(got, want);
    assert!(a.contains(stored), "another environment: {a}");
    assert_eq!((n, payloads()), (2, 2), "the same payloads stored again");
    assert!(entries().len() > all, "no entries of the new key");
    let (got, a, _) = acached(&dir, &other, &["--cycle", "--two-state"]);
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
        let flags = [&["--cycle", "--two-state"][..], flags].concat();
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
        let flags = [
            "--cycle".to_string(),
            "--two-state".into(),
            format!("-gK={k}"),
        ];
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
    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state"]);
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
        let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state"]);
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
            std::thread::spawn(move || acached(&d, &[], &["--cycle", "--two-state", "-gW=9"]).0)
        })
        .collect();
    for r in racers {
        assert_eq!(r.join().expect("racer"), w9);
    }
    let (got, a, _) = acached(&dir, &[], &["--cycle", "--two-state", "-gW=9"]);
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
            .args(["--cycle", "--two-state", "acfile"])
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
        .args(["--cycle", "--two-state", "cycle_many"])
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
        &[
            "run",
            "--work=work:lib",
            "--cycle-report",
            "--two-state",
            "cycle_codes",
        ],
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_subrange",
            );
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
            let (cycle, err) = run_vars(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_skip",
                &env,
            );
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
        for mode in [&[][..], &["--cycle", "--two-state"]] {
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
            let idx = if !mode.is_empty() {
                "index outside of"
            } else {
                "index 9 outside of"
            };
            assert!(
                err.contains(&format!("error[E3009]: {idx}"))
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
                    &[&t[..], &["--cycle", "--two-state"]].concat(),
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

/// An edge run of a set of classes that changed nothing is skipped until an input is read or
/// another run changes the image (`idle.vhd`): an idle clock while the other toggles, a register
/// that stops at its top, an input moved in the delta of an edge, between edges and an
/// asynchronous reset from outside. Against the normal mode at widths 1 to 128: with the tests'
/// checks (every run the rule skips runs and must change nothing) and with the skips made.
#[test]
fn an_edge_run_that_would_change_nothing_is_skipped() {
    let dir = tempdir("cycle-idle");
    let envs: [&[(&str, &str)]; 2] = [&[], &[("YAVS_CYCLE_SKIP_CHECK", "")]];
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("idle{w}.vhd"),
            &with_width(include_str!("cycle/idle.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_idle");
            assert_eq!(normal.len(), 377, "{w} {t:?} {normal:?}");
            for env in envs {
                let mut env = env.to_vec();
                env.push(("YAVS_CYCLE_STATS", "1"));
                let (cycle, err) = run_vars(
                    &dir,
                    &[&t[..], &["--cycle", "--two-state"]].concat(),
                    "cycle_idle",
                    &env,
                );
                assert_eq!(cycle, normal, "width {w} {t:?} {env:?}\n{err}");
                let n = |what: &str| -> u64 {
                    (err.lines())
                        .find_map(|l| l.strip_prefix(&format!("cycle stats: {what}: ")))
                        .unwrap_or_else(|| panic!("no count of {what}\n{err}"))
                        .parse()
                        .expect("a number")
                };
                // Width 1 counts `sat` round forever: nothing is idle for long.
                if w > 2 {
                    assert!(
                        n("edge runs checked that changed no word of the image") > 0,
                        "{err}"
                    );
                    let skipped = n("edge runs skipped as idle");
                    assert_eq!(skipped > 0, env.len() == 2, "{env:?}\n{err}");
                }
            }
        }
    }
}

/// A region that waits on clocks and data: a wake for an event of a clock without its edge is
/// held back in the kernel (none runs without an edge and reads nothing), the same values as the
/// normal mode; a clock with both edges, one read as data, a record-field clock beside its data.
#[test]
fn a_clock_event_without_its_edge_does_not_wake_the_region() {
    let dir = tempdir("cycle-waitfilter");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("waitfilter{w}.vhd"),
            &with_width(include_str!("cycle/waitfilter.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_waitfilter");
            assert!(normal.len() > 300, "{w} {t:?} {normal:?}");
            let env = [("YAVS_CYCLE_STATS", "1")];
            let (cycle, err) = run_vars(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_waitfilter",
                &env,
            );
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 12 of 21 processes run cycle-based"),
                "{w}: {err}"
            );
            let n = |what: &str| -> u64 {
                (err.lines())
                    .find_map(|l| l.strip_prefix(&format!("cycle stats: {what}: ")))
                    .unwrap_or_else(|| panic!("no count of {what}\n{err}"))
                    .parse()
                    .expect("a number")
            };
            assert_eq!(
                n("runs without an edge that read nothing"),
                0,
                "{w} {t:?}\n{err}"
            );
            assert!(n("runs at an edge") > 100, "{w} {t:?}\n{err}");
        }
    }
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_portact",
            );
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
    let (cycle, err) = run(&dir, &["--cycle", "--two-state"], "cycle_bigram");
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_idioms",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_nullvec",
            );
            assert_eq!(normal.len(), 20, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 6 of 8 processes run cycle-based")
                    && err.contains(&format!(
                        "cycle_nullvec.u.wr2 (nullvec{w}.vhd:40)         E4101 an index chosen at run \
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_portres",
            );
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert!(normal.iter().any(|l| l.contains('X')), "{normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 6 processes run cycle-based")
                    && err.contains(
                        "E4124 a signal with two drivers: cycle_portres.u.two is driven by"
                    ),
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_portin",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_index2",
            );
            assert_eq!(normal.len(), 16, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{w}: {err}"
            );
        }
        for (bad, msg) in [
            ("1", format!("outside of range {} downto 0 for m", w - 1)),
            ("2", "outside of range 0 to 5 for m".to_string()),
        ] {
            let mut shown = Vec::new();
            for mode in MODES {
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
            assert_eq!(shown[0], shown[2], "{w} {bad}");
            assert_eq!(bare(&shown[0]), shown[1], "{w} {bad}");
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_romrec",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_slicepc",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_regtab",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_mathreal",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_oprange",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_elemattr",
            );
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
        let flags = ["--tier=interp", "--cycle", "--two-state"];
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_recfmt",
            );
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
        let flags = ["--tier=interp", "--cycle", "--two-state"];
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
            for mode in [&[][..], &["--cycle", "--two-state"]] {
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
                if !mode.is_empty() {
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_deltacls",
            );
            let (normal, cycle) = (k(normal), k(cycle));
            assert_eq!(normal.len(), 24, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 12 of 19 processes run cycle-based")
                    && err.contains(
                        "registers on cycle_deltacls.ca feed it and a register on \
                         cycle_deltacls.cc samples it"
                    ),
                "{w}: {err}"
            );
        }
    }
    let (normal, _) = run(&dir, &[], "cycle_deltazero");
    let (cycle, err) = run(&dir, &["--cycle", "--two-state"], "cycle_deltazero");
    assert_eq!(normal.len(), 1, "{normal:?}");
    assert_eq!(cycle, normal, "{err}");
    assert!(
        err.contains("cycle mode: 3 of 7 processes run cycle-based")
            && err.contains(
                "registers on cycle_deltazero.cc feed it and a register on \
                 cycle_deltazero.cz samples it"
            )
            && err.lines().any(
                |l| l.contains("cycle_deltazero.xz <= ... (deltacls128.vhd:107)")
                    && l.ends_with("combinational")
            ),
        "{err}"
    );
}

/// A register on a copy of a clock, sent back to the copy because a register outside waits on it,
/// reads one on a sibling copy that moved onto the source (A32): the moved one goes back too, or
/// the reader takes its new value a delta early (fuzzer, seed 327 of the two-state contract).
/// Widths 1 to 128, against the normal mode, with the membership.
#[test]
fn a_register_on_a_copy_reading_a_moved_register_keeps_the_normal_value() {
    let dir = tempdir("cycle-copyback");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("copyback{w}.vhd"),
            &with_width(include_str!("cycle/copyback.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_copyback");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_copyback",
            );
            assert_eq!(normal.len(), 6, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 7 processes run cycle-based"),
                "{w}: {err}"
            );
        }
    }
}

/// The most negative value divided by -1 (fuzzer, seed 673): `rem` and `mod` give 0 and
/// `numeric_std`'s `/` the most negative value, as in the normal mode, also for a one-element -1
/// and an integer type of 64 bits; the region stopped with an overflow error. Widths 1 to 128:
/// over 64 elements the divider stays out (one-word division only).
#[test]
fn the_most_negative_value_divided_by_minus_one_equals_the_normal_mode() {
    let dir = tempdir("cycle-divmin");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("divmin{w}.vhd"),
            &with_width(include_str!("cycle/divmin.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_divmin");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_divmin",
            );
            assert_eq!(normal.len(), 8, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            let n = if w <= 64 { 1 } else { 0 };
            assert!(
                err.contains(&format!("cycle mode: {n} of 2 processes run cycle-based")),
                "{w}: {err}"
            );
        }
    }
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_wideshift",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_runcount",
            );
            assert_eq!(normal.len(), 12, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 26 processes run cycle-based")
                    && err.contains(
                        "it drives cycle_runcount.c2, which \
                         cycle_runcount.stale reads through logic"
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.c3, which \
                         cycle_runcount.dut.list reads through logic"
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.e4, which \
                         cycle_runcount.dut.mask reads through logic"
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.c6, which \
                         cycle_runcount.dut.waiter reads through logic"
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.c8, which \
                         cycle_runcount.dut.resets reads through logic"
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.c12, which \
                         cycle_runcount.dut.li <= ..."
                    )
                    && err.contains(
                        "it drives cycle_runcount.dut.c14, which \
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

/// Concurrent assignments with a delay below the top (inertial, a reject limit, transport with a
/// delay per alternative) depend on one-delta glitches of their inputs: their logic stays in the
/// event kernel, and that of an assignment without a delay joins; against the normal mode at
/// widths 1 to 128 (with the logic in the region the delayed outputs differed).
#[test]
fn logic_feeding_a_delayed_assignment_stays_out() {
    let dir = tempdir("cycle-inertial");
    for w in [1, 63, 64, 65, 128] {
        analyze(
            &dir,
            &format!("inertial{w}.vhd"),
            &with_width(include_str!("cycle/inertial.vhd"), w),
        );
        for t in tiers() {
            let (normal, _) = run(&dir, &t, "cycle_inertial");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_inertial",
            );
            assert_eq!(normal.len(), 22, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 4 of 14 processes run cycle-based"),
                "{w}: {err}"
            );
            for m in ["c1", "g2", "c3"] {
                let (_, r) = row(&err, &format!("cycle_inertial.dut.{m} <= ..."));
                assert!(r.contains("E4129"), "{m}: {r}");
            }
            for m in ["b4", "c4", "y4"] {
                let label = format!("cycle_inertial.dut.{m} <= ...");
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_knownvar",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_intcat",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&flags[..], &["--cycle", "--two-state"]].concat(),
                "cycle_shram",
            );
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
                        "E4114 a shared variable used by two processes: `{who}` is also used by cycle_shram.hid.{other}"
                    )),
                    "{who}: {err}"
                );
            }
            assert!(
                err.contains(
                    "E4114 a shared variable used by two processes: `mem` is also used by cycle_shram.tp.p_read"
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
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_strgen",
        );
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
/// whose arguments only the unrolled loop knows joins, its assert evaluated where it runs (A52); a failing call in a branch that never runs is not seen. A constant's report is made
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
        f.extend(["--cycle", "--two-state"]);
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
                f.extend(["--cycle", "--two-state"]);
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
        // A failing assert in the unrolled loop is evaluated in `pc`, which joins (A52).
        for s in ["-gs=note", "-gs=warning"] {
            let normal = go(&flags(&["-gloop_odd=true", s], false));
            let (c, out, err) = go(&flags(&["-gloop_odd=true", s], true));
            assert_eq!((c, &out), (normal.0, &normal.1), "{t:?} {s}\n{err}");
            members(&err, 9);
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
    // The failing assert of the unrolled loop is evaluated: `pc` joins (A52).
    let r = report(&["-gloop_odd=true"]);
    assert_eq!(row(&r, "cycle_foldrep.u.pc").0, "cycle-based:", "{r}");
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
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle", "--two-state"]].concat(), top);
            let got: Vec<&str> = (normal.iter())
                .map(|l| l.rsplit(": ").next().unwrap_or(""))
                .collect();
            assert_eq!(got, want, "{top} {t:?}");
            assert_eq!(cycle, normal, "{top} {t:?}\n{err}");
            assert!(
                err.contains("a signal with a resolution function of its own"),
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
            let cycle = go(&[&f[..], &["--cycle", "--two-state"]].concat());
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
            let cycle = go(&[&f[..], &["-gbad=true", "--cycle", "--two-state"]].concat());
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
                args.extend(["--cycle", "--two-state"]);
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
            let cycle = yavs(
                &dir,
                &[&args[..], &["--cycle", "--two-state", "cycle_undriven"]].concat(),
            );
            let report = yavs(
                &dir,
                &[
                    &args[..],
                    &["--cycle-report", "--two-state", "cycle_undriven"],
                ]
                .concat(),
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
            let exact = ["cycle_metaout"];
            let cycle = yavs(&dir, &[&args[..], &["--cycle"], &exact].concat());
            let report = yavs(&dir, &[&args[..], &["--cycle-report"], &exact].concat());
            // The default: the members join and give '0' for the metavalues (A31).
            let two = yavs(
                &dir,
                &[&args[..], &["--cycle", "--two-state", "cycle_metaout"]].concat(),
            );
            let two_rep = yavs(
                &dir,
                &[
                    &args[..],
                    &["--cycle-report", "--two-state", "cycle_metaout"],
                ]
                .concat(),
            );
            let two_rep = String::from_utf8_lossy(&two_rep.stderr);
            assert!(
                two_rep.contains("cycle mode: 6 of 10 processes run cycle-based"),
                "{two_rep}"
            );
            assert_eq!(
                (
                    two.status.code(),
                    String::from_utf8_lossy(&two.stdout).into_owned()
                ),
                (
                    normal.status.code(),
                    (String::from_utf8_lossy(&normal.stdout))
                        .replace("'Z'", "'0'")
                        .replace('X', "0")
                        .replace("x=true", "x=false")
                ),
                "{w} {t:?}\n{two_rep}"
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_shiftop",
            );
            assert_eq!(normal.len() as u32, 2 * w + 4, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 2 of 4 processes run cycle-based")
                    && err.contains("whose subtype allows negative values"),
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_qualrec",
            );
            assert_eq!(normal.len(), 21, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 3 of 5 processes run cycle-based")
                    && err.contains("can lie outside their subtypes"),
                "{w}: {err}"
            );
            let mut shown = Vec::new();
            for mode in [&[][..], &["--cycle", "--two-state"]] {
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_elabcall",
            );
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
            for mode in [&[][..], &["--cycle", "--two-state"]] {
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
            let (cycle, err) = run(
                &dir,
                &[&one[..], &["--cycle", "--two-state"]].concat(),
                "cycle_shiftneg",
            );
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
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_dclkreg",
            );
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 5 processes run cycle-based")
                    && err.contains("a register on cycle_dclkreg.dk samples it"),
                "{w}: {err}"
            );
        }
    }
}

/// A register outside the region on a clock copied twice samples combinational logic fed by a
/// member register on the once-copied clock: the normal mode's new value comes a delta after
/// the edge. The register stays out only for a later rule (an output port associated in parts):
/// the cross-delta rule, run in every round, keeps that logic out (found by the fuzzer).
#[test]
fn a_register_left_out_late_still_keeps_the_logic_it_samples_out() {
    let dir = tempdir("cycle-latecone");
    analyze(&dir, "latecone.vhd", include_str!("cycle/latecone.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_latecone");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_latecone",
            );
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 7 processes run cycle-based")
                    && err.contains("a register on cycle_latecone.dk2 samples it"),
                "{w}: {err}"
            );
        }
    }
}

/// Logic between registers of clocks whose edges come in different deltas, on paths through a
/// process outside the region (its history read, a metavalue literal under the exact contract),
/// from a register outside, and through a converted port: it stays out in both contracts, and
/// logic between registers of one clock through such a process joins. Two gates of a clock in
/// the region would give its edge a delta early: the second stays out. A logger with text I/O and
/// a checker of the top-level unit on the late clock sample such logic too.
#[test]
fn paths_through_processes_outside_still_keep_cross_delta_logic_out() {
    let dir = tempdir("cycle-crossout");
    analyze(&dir, "crossout.vhd", include_str!("cycle/crossout.vhd"));
    let exact = [("YAVS_CYCLE_EXACT_CLOCKS", "1")];
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            // Only the report lines: at width 1 the additions warn of truncation.
            let k = |v: Vec<String>| {
                (v.into_iter())
                    .filter(|l| [" k=", "lg=", " ck="].iter().any(|x| l.contains(x)))
                    .collect::<Vec<_>>()
            };
            let normal = k(run(&dir, &t, "cycle_xout").0);
            let gated = k(run(&dir, &t, "cycle_xgate").0);
            let sinks = k(run(&dir, &t, "cycle_xsink").0);
            assert_eq!(sinks.len(), 22, "{w} {t:?} {sinks:?}");
            assert_eq!(
                normal.len() + gated.len(),
                20,
                "{w} {t:?} {normal:?} {gated:?}"
            );
            for (flags, env) in [
                (&["--cycle", "--two-state"][..], &[][..]),
                (&["--cycle"][..], &exact[..]),
            ] {
                let f = [&t[..], flags].concat();
                let (cycle, err) = run_vars(&dir, &f, "cycle_xout", env);
                assert_eq!(k(cycle), normal, "width {w} {f:?}\n{err}");
                assert!(
                    err.contains("cycle mode: 8 of 22 processes run cycle-based"),
                    "{w}: {err}"
                );
                for p in ["xa", "z", "m", "xs2", "xq", "u.q"] {
                    assert!(
                        err.contains(&format!(
                            "cycle_xout.{p} <= ... (crossout.vhd:{}) ",
                            match p {
                                "xa" => 37,
                                "z" => 39,
                                "m" => 47,
                                "xs2" => 52,
                                "xq" => 55,
                                _ => 15,
                            }
                        )) && err.contains("a register on cycle_xout.cg2 samples it"),
                        "{p} {w}: {err}"
                    );
                }
                assert!(err.contains("    cycle_xout.xb <= ... (crossout.vhd:42)   combinational"));
                let (cycle, err) = run_vars(&dir, &f, "cycle_xgate", env);
                assert_eq!(k(cycle), gated, "width {w} {f:?}\n{err}");
                let (cycle, serr) = run_vars(&dir, &f, "cycle_xsink", env);
                assert_eq!(k(cycle), sinks, "width {w} {f:?}\n{serr}");
                assert!(
                    serr.contains("cycle mode: 2 of 11 processes run cycle-based")
                        && serr.contains("a register on cycle_xsink.cg2 samples it"),
                    "{w}: {serr}"
                );
                assert!(
                    err.contains("cycle mode: 3 of 9 processes run cycle-based")
                        && err.contains("cycle_xgate.cg1 <= ... (crossout.vhd:91)  E4131 ")
                        && err.contains(
                            "    cycle_xgate.cg0 <= ... (crossout.vhd:90)  combinational"
                        ),
                    "{w}: {err}"
                );
            }
        }
    }
}

/// A register with an asynchronous reset outside the region (the checker fits it, the lowering
/// takes no `while` loop) on a copy of the clock samples logic fed by a member register on the
/// clock: the cross-delta rule keeps that logic out as for a plain register (found by the fuzzer).
#[test]
fn a_register_with_a_reset_on_a_late_clock_keeps_the_logic_it_samples_out() {
    let dir = tempdir("cycle-lateasync");
    analyze(&dir, "lateasync.vhd", include_str!("cycle/lateasync.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_lateasync");
            let (cycle, err) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_lateasync",
            );
            assert_eq!(normal.len(), 40, "{w} {t:?} {normal:?}");
            assert_eq!(cycle, normal, "width {w} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 6 processes run cycle-based")
                    && err.contains("a register on cycle_lateasync.dk samples it"),
                "{w}: {err}"
            );
        }
    }
}

/// `--cycle-strict` on `top` of a design analysed in `dir`: exit 2, nothing simulated, and the
/// standard error with the directory written as `$DIR`.
fn strict_err(dir: &Path, top: &str, extra: &[&str]) -> String {
    let o = yavs(
        dir,
        &[
            &["run", "--work=work:lib", "--no-cache", "--cycle-strict"],
            extra,
            &[top],
        ]
        .concat(),
    );
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(2), "{top}: {err}");
    assert!(o.stdout.is_empty(), "{top}: nothing runs");
    let canon = dir.canonicalize().expect("dir");
    err.replace(&*canon.to_string_lossy(), "$DIR")
        .replace(&*dir.to_string_lossy(), "$DIR")
        .replace(r"$DIR\", "$DIR/")
}

/// Compare `err` with the snapshot `tests/cycle/strict/NAME.stderr` (`YAVS_BLESS=1` writes it).
fn strict_snapshot(name: &str, err: &str) {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/cycle/strict/{name}.stderr"));
    if std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
        std::fs::write(&path, err).expect("bless");
    } else {
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            err, want,
            "{name}: the snapshot differs (YAVS_BLESS=1 rewrites it)"
        );
    }
}

/// A40: each reason under `--cycle-strict` is an error with its own code that says what is not
/// supported (the headline), where (the statement, the other process when it spans processes,
/// the processes it keeps out), why, and how to fix it or that it is a limitation of yavs. One
/// minimal design per code, the whole message a snapshot.
#[test]
fn cycle_strict_gives_each_code_with_what_where_why_and_fix() {
    let dir = tempdir("cycle-strict-each");
    analyze(&dir, "strict.vhd", include_str!("cycle/strict.vhd"));
    let cases: &[(&str, &[&str])] = &[
        ("E4101", &["--two-state"]),
        ("E4110", &["--two-state"]),
        ("E4111", &["--two-state"]),
        ("E4112", &["--two-state"]),
        ("E4113", &["--two-state"]),
        ("E4114", &["--two-state"]),
        ("E4115", &["--two-state"]),
        ("E4117", &["--two-state"]),
        ("E4118", &["--two-state"]),
        ("E4119", &["--two-state"]),
        ("E4120", &["--two-state"]),
        ("E4121", &["--two-state"]),
        ("E4122", &["--two-state"]),
        ("E4123", &["--two-state"]),
        ("E4124", &["--two-state"]),
        ("E4125", &["--two-state"]),
        ("E4126", &["--two-state"]),
        ("E4127", &["--two-state"]),
        ("E4129", &["--two-state"]),
        ("E4130", &["--two-state"]),
        ("E4131", &["--two-state"]),
        ("E4132", &["--two-state"]),
        ("E4133", &["--two-state"]),
        ("E4134", &["--two-state"]),
        ("E4135", &["--two-state"]),
        ("E4137", &[]),
        ("E4138", &["--two-state"]),
        ("E4139", &["--two-state"]),
    ];
    for &(code, extra) in cases {
        let err = strict_err(&dir, &format!("t_{}", code.to_lowercase()), extra);
        let head = format!("error[{code}]: ");
        let at = err.find(&head).unwrap_or_else(|| panic!("{code}: {err}"));
        let first = err[at..].lines().next().expect("a line");
        assert!(
            first.ends_with(" is not supported in cycle mode"),
            "{first}"
        );
        for part in [
            "--> $DIR/strict.vhd:",
            "= note: in process",
            "= note: why: ",
            "= help: ",
        ] {
            assert!(err[at..].contains(part), "{code}: no {part:?}: {err}");
        }
        strict_snapshot(code, &err);
    }
    // A force in the design: E4116 at the force, E4128 for the driver of what it forces.
    let dir = tempdir("cycle-strict-force");
    analyze(
        &dir,
        "strictforce.vhd",
        include_str!("cycle/strictforce.vhd"),
    );
    let err = strict_err(&dir, "t_e4116", &["--two-state"]);
    assert!(
        err.contains("error[E4116]: ") && err.contains("error[E4128]: "),
        "{err}"
    );
    strict_snapshot("E4116-E4128", &err);
}

/// The reasons of constructs few designs use (`tests/cycle/rare.vhd`): each process's code and
/// text in the region report, and under `--cycle-strict` its error headline (A40).
#[test]
fn rare_reasons_are_named_in_the_report_and_under_strict() {
    let dir = tempdir("cycle-rare");
    std::fs::write(dir.join("rare.vhd"), include_str!("cycle/rare.vhd")).expect("write");
    let o = yavs(
        &dir,
        &[
            "analyze",
            "--relaxed",
            "--std=2008",
            "--work=work:lib",
            "rare.vhd",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // Per top: process, code and text.
    type Rows<'a> = &'a [(&'a str, &'a str, &'a str)];
    let cases: [(&str, Rows); 2] = [
        (
            "rare_a_tb",
            &[
                (
                    "dut.implicit",
                    "E4112",
                    "an implicit signal (`'stable`, `'quiet`, `'delayed`, `'transaction`)",
                ),
                (
                    "dut.g.guard_read",
                    "E4115",
                    "the `GUARD` signal of a guarded block",
                ),
                ("dut.shared_comb", "E4114", "the shared variable `sv`"),
                (
                    "dut.access_val",
                    "E4114",
                    "an access value (dynamic memory)",
                ),
                (
                    "dut.clock_at_index",
                    "E4123",
                    "a clocked process of another shape: a clock or reset that is not one fixed element of a signal",
                ),
                (
                    "dut.call_force",
                    "E4116",
                    "a call of `pforce`: it forces a signal",
                ),
                (
                    "dut.call_delay",
                    "E4111",
                    "a call of `pdelay`: it assigns a signal with a delay",
                ),
                (
                    "dut.call_edge",
                    "E4112",
                    "a call of `fedge`: it tests a signal edge",
                ),
                (
                    "dut.call_prot",
                    "E4114",
                    "a call of `fprot`: it uses a protected object",
                ),
                (
                    "dut.mem_both",
                    "E4133",
                    "a memory written both at run-time indices and at fixed ones: rare_a_tb.dut.mem",
                ),
                (
                    "dut.mem_out",
                    "E4133",
                    "a memory read outside the region: rare_a_tb.dut.imem",
                ),
                (
                    "dut.call_wait",
                    "E4110",
                    "a call of `pause_if`, which waits",
                ),
            ],
        ),
        (
            "rare_b_tb",
            &[
                (
                    "dut.val_logic",
                    "E4101",
                    "`'val` of a logic type at a position computed at run time (it can be a metavalue)",
                ),
                (
                    "dut.int_wide",
                    "E4101",
                    "`to_integer` of a vector of more than 64 elements",
                ),
                (
                    "dut.x01_sig",
                    "E4101",
                    "a logic subtype with a range (`X01`, `UX01`, ...) (signal `x01s`)",
                ),
                (
                    "dut.deep",
                    "E4101",
                    "an array nested more than four deep (signal `n5`)",
                ),
                (
                    "dut.int_conv",
                    "E4101",
                    "a type conversion that is not between logic vector types",
                ),
                (
                    "dut.no_return",
                    "E4101",
                    "a function `noret` that can reach its end without `return`",
                ),
                (
                    "dut.outside_formal",
                    "E4101",
                    "a value outside the subtype of parameter N",
                ),
                (
                    "dut.individual",
                    "E4101",
                    "a call with an open, converted or individual actual",
                ),
                ("dut.succ", "E4101", "an attribute of this kind"),
                (
                    "dut.pkg_shared",
                    "E4101",
                    "the shared variable `psv`, which is not of an architecture or block around the process",
                ),
                (
                    "dut.recursion",
                    "E4101",
                    "calls of `fact` nested more than 32 deep (a recursion elaboration does not bound)",
                ),
                (
                    "dut.short_value",
                    "E4101",
                    "an assignment of a value of another length than its target",
                ),
                (
                    "dut.short_gate",
                    "E4101",
                    "a logical operator on operands of different lengths",
                ),
                (
                    "dut.short_match",
                    "E4101",
                    "a matching comparison of operands of different lengths",
                ),
                (
                    "dut.short_formal",
                    "E4101",
                    "a value of another length than its subtype",
                ),
                (
                    "dut.slice_dir",
                    "E4101",
                    "a slice in the other direction than its array",
                ),
                (
                    "dut.slice_out",
                    "E4101",
                    "a slice outside its array's range",
                ),
                (
                    "dut.huge",
                    "E4101",
                    "a vector of 134217728 elements (cycle mode takes 1 to 2**26) (variable `big`)",
                ),
                (
                    "dut.neg_count",
                    "E4101",
                    "a shift operator by a count chosen at run time whose subtype allows negative values",
                ),
                (
                    "dut.two_conds",
                    "E4101",
                    "a clocked process whose condition is more than the edge test and one more condition",
                ),
                (
                    "dut.pieces",
                    "E4101",
                    "an aggregate with slice pieces without a range choice",
                ),
                ("dut.null_size", "E4101", "a null or very large vector size"),
                (
                    "dut.neg_known",
                    "E4101",
                    "a shift by a negative count known during elaboration",
                ),
                (
                    "dut.choice_len",
                    "E4101",
                    "a `case` choice of another length than the selector",
                ),
                (
                    "dut.sel <= ...",
                    "E4101",
                    "a `case` choice that is not known during elaboration",
                ),
                // The top-level unit's checkers stay in the event kernel.
                (
                    "sev_signal",
                    "",
                    "a severity that is not known during elaboration",
                ),
                (
                    "real_image",
                    "",
                    "a value of this type in a report message, which cycle mode cannot format in region code",
                ),
                ("hex_signed", "", "a report message of this form"),
            ],
        ),
    ];
    for (top, rows) in cases {
        let o = yavs(
            &dir,
            &[
                "run",
                "--relaxed",
                "--work=work:lib",
                "--no-cache",
                "--cycle-report",
                "--two-state",
                top,
            ],
        );
        let report = String::from_utf8_lossy(&o.stderr).into_owned();
        assert!(o.status.success(), "{report}");
        let o = yavs(
            &dir,
            &[
                "run",
                "--relaxed",
                "--work=work:lib",
                "--no-cache",
                "--cycle-strict",
                "--two-state",
                top,
            ],
        );
        let strict = String::from_utf8_lossy(&o.stderr).into_owned();
        assert_eq!(o.status.code(), Some(2), "{strict}");
        for (label, code, text) in rows {
            let (_, r) = row(&report, &format!("{top}.{label}"));
            assert!(
                r.contains(&format!("{code} {text}").trim_start().to_string()),
                "{r}"
            );
            if code.is_empty() {
                continue;
            }
            // The headline is the text up to its first `: ` outside backticks.
            let mut quoted = false;
            let cut = (text.char_indices())
                .find(|&(i, c)| {
                    quoted ^= c == '`';
                    !quoted && c == ':' && text[i + 1..].starts_with(' ')
                })
                .map_or(text.len(), |(i, _)| i);
            let head = format!(
                "error[{code}]: {} is not supported in cycle mode",
                &text[..cut]
            );
            assert!(strict.contains(&head), "{head}\n{strict}");
        }
    }
}

/// Strict lists every reason at once, one error per statement, sorted by file and line, with
/// the processes of a statement used by several instances counted in one error and a cap of 40
/// errors with a line saying how many more there are.
#[test]
fn cycle_strict_lists_every_reason_sorted_grouped_and_capped() {
    let dir = tempdir("cycle-strict-many");
    analyze(&dir, "subset.vhd", include_str!("cycle/subset.vhd"));
    let err = strict_err(&dir, "sub_tb", &["--two-state"]);
    let lines: Vec<usize> = (err.lines())
        .filter_map(|l| l.trim_start().strip_prefix("--> $DIR/subset.vhd:"))
        .map(|l| {
            l.split(':')
                .next()
                .expect("line")
                .parse()
                .expect("a number")
        })
        .collect();
    assert_eq!(lines.len(), 40, "{err}");
    assert!(lines.is_sorted(), "{lines:?}");
    let codes: std::collections::BTreeSet<&str> = (err.lines())
        .filter_map(|l| l.strip_prefix("error[E41"))
        .map(|l| &l[..2])
        .collect();
    assert!(codes.len() >= 15, "{codes:?}");
    assert!(
        err.contains("more errors not shown; `--cycle-report` lists every process"),
        "{err}"
    );
    assert!(
        err.contains("error: aborting due to 40 previous errors"),
        "{err}"
    );
    // One statement, several instances: one error naming them all.
    let dir = tempdir("cycle-strict-group");
    analyze(
        &dir,
        "strict.vhd",
        &(include_str!("cycle/strict.vhd").to_string() + GROUPED),
    );
    let err = strict_err(&dir, "t_many", &["--two-state"]);
    assert_eq!(err.matches("error[E4111]").count(), 1, "{err}");
    // Also when the text names each instance's signal: the first one's.
    assert_eq!(err.matches("error[E4119]").count(), 1, "{err}");
    assert!(
        err.contains("in 7 processes: t_many.g(0).latch.p (strict.vhd:"),
        "{err}"
    );
    assert!(
        err.contains("= note: in 7 processes: t_many.g(0).dut.q <= ... (strict.vhd:")
            && err.contains(" and 2 more"),
        "{err}"
    );
}

const GROUPED: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity t_many is
end entity;
architecture tb of t_many is
  signal a : std_ulogic := '0';
  signal q, r : std_ulogic_vector(0 to 6);
begin
  g : for i in 0 to 6 generate
    dut : entity work.d_e4111 port map (a, q(i));
    latch : entity work.d_e4119 port map (a, a, r(i));
  end generate;
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;
";

/// Reasons of one code with other wording (A40): each process of `strictmore.vhd` gives the row
/// of its reason in the region report, the same words as its error under `--cycle-strict`.
#[test]
fn cycle_strict_reason_variants_are_triggered() {
    let dir = tempdir("cycle-strict-more");
    analyze(&dir, "strictmore.vhd", include_str!("cycle/strictmore.vhd"));
    let report = verdicts(&dir, "t_more");
    for (label, want) in [
        (
            "reset_reads_clock",
            "E4123 a clocked process of another shape: the asynchronous reset condition reads the clock (strictmore.vhd:16)",
        ),
        (
            "reset_no_signal",
            "E4123 a clocked process of another shape: the branch before the edge test tests no signal, so it is not an asynchronous reset (strictmore.vhd:24)",
        ),
        (
            "next_outer",
            "E4139 a `next` of an outer loop (strictmore.vhd:36)",
        ),
        (
            "no_exit",
            "E4139 a loop without an exit (strictmore.vhd:48)",
        ),
        (
            "big_loops",
            "E4101 a loop of 300 iterations, more than the 65536 statements cycle mode unrolls (strictmore.vhd:59)",
        ),
        (
            "guarded_null.nul",
            "E4115 a `null` waveform (strictmore.vhd:79)",
        ),
        (
            "guarded_null.GUARD <= ...",
            "E4115 a guarded block (strictmore.vhd:74)",
        ),
        (
            "dyn_loop",
            "E4101 a value that is not known during elaboration (`to_integer(unsigned(v))`) (strictmore.vhd:90)",
        ),
    ] {
        let (_, r) = row(&report, &format!("t_more.dut.{label}"));
        assert!(r.ends_with(want), "{label}: {r}");
    }
    let err = strict_err(&dir, "t_more", &["--two-state"]);
    assert!(
        err.contains("error[E4139]: a `next` of an outer loop is not supported in cycle mode"),
        "{err}"
    );
}

/// 'U' at start without `--two-state` (owner decision A38): the second image shows the
/// testbench 'U' where the normal mode has it, from registers and a clocked variable without a
/// start value, a vector whose high word alone stays 'U', an input that becomes 'U' after time 0,
/// outputs read with and without a wait on them, and `BIT` and `BOOLEAN` outputs (also fields
/// of a record with a `std_ulogic` one), never 'U'; a register whose enable is 'U' keeps its
/// value as in the normal mode, not 'U'; a register that keeps 'U' gives '0' from
/// the cap of 1024 clock edges on. The two-state contract (`--cycle --two-state`) and `YAVS_CYCLE_RAIL=off`
/// give '0' throughout, and an event at 0 fs from 'U' to '0' the normal mode does not have.
/// Widths 1 to 128.
#[test]
fn u_at_start_is_followed_on_a_second_image() {
    let dir = tempdir("cycle-rail");
    analyze(&dir, "rail.vhd", include_str!("cycle/rail.vhd"));
    let zero = |l: &[String]| -> Vec<String> { l.iter().map(|x| x.replace('U', "0")).collect() };
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let (normal, _) = run(&dir, &t, "cycle_rail");
            assert_eq!(normal.len(), 13, "{normal:?}");
            let exact = [&t[..], &["--cycle"]].concat();
            let (cycle, err) = run(&dir, &exact, "cycle_rail");
            // Up to the cap the normal mode's lines; `held` gives '0' after it.
            let k = normal.len() - 1;
            assert!(
                normal[k].contains(" k ") && normal[k].contains(&"U".repeat(w)),
                "{normal:?}"
            );
            assert_eq!(cycle[..k], normal[..k], "width {w} {t:?}\n{err}");
            let (head, held) = normal[k].split_once("held=").expect("held");
            assert_eq!(
                cycle[k],
                format!("{head}held={}", held.replace('U', "0")),
                "{w}"
            );
            assert!(
                err.contains("cycle mode: 6 of 9 processes run cycle-based")
                    && err.contains("second image: stopped at the cap of 1024 clock edges"),
                "{w}: {err}"
            );
            for p in ["pipe", "hp", "vp", "cpp", "hold", "bp"] {
                let p = format!("    cycle_rail.{p} ");
                assert!(err.lines().any(|l| l.starts_with(&p)), "{p}: {err}");
            }
            // Without the second image `s3` starts 'U' and is driven '0' a delta later.
            let mut want = zero(&normal);
            want.insert(0, normal[0].clone());
            want[1] = want[1].replace("0fs+0", "0fs+1");
            let (two, _) = run(
                &dir,
                &[&t[..], &["--cycle", "--two-state"]].concat(),
                "cycle_rail",
            );
            assert_eq!(two, want, "{w} {t:?}");
            let (off, err) = run_vars(&dir, &exact, "cycle_rail", &[("YAVS_CYCLE_RAIL", "off")]);
            assert_eq!(off, want, "{w} {t:?}");
            assert!(!err.contains("second image"), "{err}");
            // The knob: no edge is the same as off; a cap of 5 gives `held` '0' from the
            // first lines on; without the inputs set at import `cp` reads '0' after 100 ns.
            let knob = |v: &str| run_vars(&dir, &exact, "cycle_rail", &[("YAVS_CYCLE_RAIL", v)]).0;
            assert_eq!(knob("cap=0"), want, "{w}");
            let five = knob("cap=5");
            assert_eq!(five[..3], cycle[..3], "{w}");
            assert!(
                five[9].contains(&format!("held={}", "0".repeat(w))),
                "{five:?}"
            );
            let no = knob("no-reseed");
            assert!(
                cycle[9].contains(&format!("cp={}", "U".repeat(w))),
                "{cycle:?}"
            );
            assert_eq!(
                no[9],
                cycle[9].replace(
                    &format!("cp={}", "U".repeat(w)),
                    &format!("cp={}", "0".repeat(w))
                ),
                "{w}"
            );
            // Where every 'U' is overwritten the images agree and the run is the normal mode's.
            let hold = [&exact[..], &["-gKEEP=false"]].concat();
            let (n2, _) = run(&dir, &[&t[..], &["-gKEEP=false"]].concat(), "cycle_rail");
            let (c2, err) = run(&dir, &hold, "cycle_rail");
            assert_eq!(c2, n2, "{w} {t:?}");
            assert!(
                err.contains("second image: the images agreed after 32 clock edges"),
                "{err}"
            );
        }
    }
}

/// The second image's region is the one the caches keep: a warm run (each hit compiled again
/// and compared) gives what a cold one gives.
#[test]
fn u_at_start_from_the_caches_gives_the_same_result() {
    let dir = tempdir("cycle-rail-cache");
    analyze(&dir, "rail.vhd", include_str!("cycle/rail.vhd"));
    let args = ["run", "--work=work:lib", "-gW=65", "--cycle", "cycle_rail"];
    let cold = yavs(&dir, &args);
    let warm = yavs_env(&dir, &args, &[("YAVS_CYCLE_CACHE_CHECK", "1")]);
    let again = yavs_env(&dir, &args, &[("YAVS_CYCLE_STATS", "1")]);
    for o in [&cold, &warm, &again] {
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(o.stdout, cold.stdout);
    }
    let err = String::from_utf8_lossy(&again.stderr);
    assert!(err.contains("cycle stats: region from the cache"), "{err}");
    assert!(String::from_utf8_lossy(&cold.stdout).contains("hi=U"));
}

/// A check that fails on the second image alone ends it and never the run: at initialisation
/// the run is that of `YAVS_CYCLE_RAIL=off`; at a later wake the 'U' shown before stays, the
/// outputs are driven from the region's values from there on, and the summary names the check.
#[test]
fn a_check_failing_on_the_second_image_ends_it() {
    let dir = tempdir("cycle-railchk");
    analyze(&dir, "railchk.vhd", include_str!("cycle/railchk.vhd"));
    for w in [1, 63, 64, 65, 128] {
        for mid in [false, true] {
            let t = [format!("-gW={w}"), format!("-gMID={mid}")];
            let t = [t[0].as_str(), t[1].as_str(), "--cycle"];
            let (normal, _) = run(&dir, &t[..2], "cycle_railchk");
            let (cycle, err) = run(&dir, &t, "cycle_railchk");
            let (off, _) = run_vars(&dir, &t, "cycle_railchk", &[("YAVS_CYCLE_RAIL", "off")]);
            assert_eq!(normal.len(), 3, "{normal:?}");
            assert!(
                normal[0].contains("r=U") && normal[1].contains("r=U"),
                "{normal:?}"
            );
            if mid {
                assert_eq!(cycle[0], normal[0], "{w}");
                assert_eq!(cycle[1..], off[1..], "{w}");
            } else {
                assert_eq!(cycle, off, "{w}");
            }
            assert!(
                err.contains("cycle mode: 2 of 4 processes run cycle-based")
                    && err.contains(&format!(
                        "stopped after {}: a check failed on it in cycle_railchk.ix (value 1 \
                         outside of INTEGER range 0 to 0 for signal IDX)",
                        if mid { "1 clock edge" } else { "0 clock edges" }
                    )),
                "{w} {mid}: {err}"
            );
        }
    }
}

/// `YAVS_CYCLE_CACHE_MIB` takes a whole number of MiB of at least 1; another value is an error
/// before the run.
#[test]
fn the_cycle_store_size_takes_mib() {
    let dir = tempdir("cycle-storemib");
    analyze(&dir, "rail.vhd", include_str!("cycle/rail.vhd"));
    let args = [
        "run",
        "--work=work:lib",
        "--cycle",
        "--two-state",
        "cycle_rail",
    ];
    for v in ["0", "2G", "-1", "99999999999999999"] {
        let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_CACHE_MIB", v)]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!o.status.success() && o.stdout.is_empty(), "{v}: {err}");
        assert!(
            err.contains("error[E4104]: YAVS_CYCLE_CACHE_MIB takes a whole number of MiB"),
            "{err}"
        );
    }
    for v in ["", "1", "4096"] {
        let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_CACHE_MIB", v)]);
        assert!(
            o.status.success(),
            "{v}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

/// The second image follows memories, the locals of an inlined call, 'X' and choices of
/// combinational members on 'U' (owner decision A43): a memory word never written and one
/// written from an input that becomes 'U' show 'U', a local of a call starts 'U', `when ... else`
/// (also on `??`) and `to_integer` as an index in combinational members take the region's
/// choice, which is the normal mode's here (no check fails on the second image), and a
/// comparison of a value only the second image got wrong (`xx`, chosen on a variable) still
/// gives the normal mode's result (`zz`); an 'X' start value or input shows 'X', a mix with 'U'
/// 'U'. Clocked members read their own image (taking the region's choices there made a 'U' in
/// Microwatt the normal mode does not have): their choices on a 'U' give 'U' (`c`, `r`, `lt`,
/// `r0`, `lt0`, `z` before `t` has a value) and, as they keep registers (`rail_keep`), a
/// register taking an input's 'U' while its own value stays (`q`) or an 'X' (`qx`, 'U') loses
/// it. `YAVS_CYCLE_RAIL=off` gives '0' for every metavalue. Widths 1 to 128.
#[test]
fn the_second_image_follows_memories_calls_and_choices() {
    let dir = tempdir("cycle-rail2");
    analyze(&dir, "rail2.vhd", include_str!("cycle/rail2.vhd"));
    for (w, late) in [1, 63, 64, 65, 128]
        .into_iter()
        .flat_map(|w| [(w, false), (w, true)])
    {
        let g = [format!("-gW={w}"), format!("-gLATE={late}")];
        for t in tiers() {
            let t = [&t[..], &[&g[0][..], &g[1][..]]].concat();
            // The notes (a library warning's process line has an `=` too).
            let notes = |l: Vec<String>| -> Vec<String> {
                l.into_iter().filter(|x| x.contains(" m0=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_rail2");
            let normal = notes(normal);
            assert_eq!(normal.len(), 3, "{normal:?}");
            let exact = [&t[..], &["--cycle"]].concat();
            let (cycle, err) = run(&dir, &exact, "cycle_rail2");
            let cycle = notes(cycle);
            assert_eq!(cycle.len(), 3, "{cycle:?}");
            let (zero, one, u) = ("0".repeat(w), "1".repeat(w), "U".repeat(w));
            let field = |l: &str, k: &str| -> String {
                let at = l.find(&format!(" {k}=")).expect(k) + k.len() + 2;
                l[at..].split(' ').next().unwrap_or_default().to_string()
            };
            let same = [
                "m0", "m1", "f", "y", "y2", "w2", "rd", "qa", "qo", "k", "k2", "qxi", "zz",
            ];
            for (i, (n, c)) in normal.iter().zip(&cycle).enumerate() {
                for k in same {
                    assert_eq!(field(c, k), field(n, k), "width {w} {k}: {c}\n{n}\n{err}");
                }
                for k in ["c", "r", "lt", "r0", "lt0", "xx", "qx"] {
                    assert_eq!(field(c, k), u, "{w} {k}: {c}");
                }
                assert_eq!(field(c, "q"), zero, "{c}");
                assert_eq!(&field(c, "z"), if i < 2 { &u } else { &one }, "{c}");
            }
            // The normal mode's values of those fields.
            let b = &normal[2];
            let mut m1 = format!("{}U", "1".repeat(w - 1));
            assert_eq!(field(b, "m1"), m1, "{b}");
            assert_eq!(field(&normal[0], "m1"), u);
            assert_eq!(field(b, "f"), format!("{}1", "U".repeat(w - 1)));
            assert_eq!(field(b, "q"), format!("{}U", "0".repeat(w - 1)));
            for (k, v) in [
                ("y", &zero),
                ("y2", &zero),
                ("w2", &one),
                ("rd", &one),
                ("xx", &zero),
                ("zz", &one),
                ("qa", &u),
                ("qo", &u),
                ("k", &zero),
                ("k2", &zero),
                ("z", &one),
            ] {
                assert_eq!(&field(b, k), v, "{k}: {b}");
            }
            let xs = "X".repeat(w);
            assert_eq!(field(b, "qx"), if late { &u } else { &xs }.clone());
            assert_eq!(field(b, "qxi"), if late { &xs } else { &zero }.clone());
            assert!(
                err.contains("cycle mode: 10 of 12 processes run cycle-based")
                    && err.contains("second image: still followed when the run ended"),
                "{w}: {err}"
            );
            for p in ["wr", "ch", "kp", "xp", "cv"] {
                let p = format!("    cycle_rail2.{p} ");
                assert!(err.lines().any(|l| l.starts_with(&p)), "{p}: {err}");
            }
            let (off, _) = run_vars(&dir, &exact, "cycle_rail2", &[("YAVS_CYCLE_RAIL", "off")]);
            let off = notes(off);
            m1.pop();
            m1.push('0');
            let ob = &off[2];
            assert_eq!(field(ob, "m1"), m1, "{ob}");
            for k in ["f", "q", "qx", "qa", "qo", "qxi", "xx", "c"] {
                assert!(!field(ob, k).contains(['U', 'X']), "{k}: {ob}");
            }
        }
    }
}

/// Clocked members choose on each image's own values without `--two-state`: where the
/// region's image reads a 'U' as 0 and so takes a branch the normal mode does not (`s = '0'`,
/// `unsigned(ad) < 2`, `when "00"`), the second image takes the normal mode's, and a register
/// loaded from a 'U' there shows the normal mode's value, not 'U' (A43: following the region's
/// branch in clocked members failed an assert in Microwatt's icache). Widths 1 to 128.
#[test]
fn clocked_choices_on_a_u_take_each_images_own_branch() {
    let dir = tempdir("cycle-railclk");
    analyze(&dir, "railclk.vhd", include_str!("cycle/railclk.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let notes = |l: Vec<String>| -> Vec<String> {
                l.into_iter().filter(|x| x.contains(" qn=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_railclk");
            let normal = notes(normal);
            let zero = "0".repeat(w);
            let want = format!("q0={zero} qn={zero} qc={zero}");
            assert!(
                normal.len() == 1 && normal[0].ends_with(&want),
                "{normal:?}"
            );
            let exact = [&t[..], &["--cycle"]].concat();
            let (cycle, err) = run(&dir, &exact, "cycle_railclk");
            let cycle = notes(cycle);
            assert!(
                cycle.len() == 1 && cycle[0].ends_with(&want),
                "{w}: {cycle:?}\n{err}"
            );
            assert!(
                err.contains("cycle mode: 1 of 3 processes run cycle-based")
                    && err.contains("second image: still followed when the run ended")
                    && err.lines().any(|l| l.starts_with("    cycle_railclk.ck ")),
                "{w}: {err}"
            );
        }
    }
}

/// A guarded combinational member that reads the region's image runs again on the second image
/// when only the region's image changed: `y <= a when s = '1' else b` reads `s` as the region's
/// image holds it, and when `s` goes from 'U' to '1' (already 1 on the second image) `y` and `q`
/// show `a` as in the normal mode (with every step guarded). Widths 1 to 128.
#[test]
fn the_second_image_runs_a_member_that_reads_the_regions_image() {
    let dir = tempdir("cycle-railguard");
    analyze(&dir, "railguard.vhd", include_str!("cycle/railguard.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let notes = |l: Vec<String>| -> Vec<String> {
                l.into_iter().filter(|x| x.contains("y=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_railguard");
            let normal = notes(normal);
            let (zero, one) = ("0".repeat(w), "1".repeat(w));
            assert!(
                normal.len() == 2 && normal[0].ends_with(&format!("y={zero}")),
                "{normal:?}"
            );
            assert!(
                normal[1].ends_with(&format!("y={one} q={one}")),
                "{normal:?}"
            );
            let exact = [&t[..], &["--cycle"]].concat();
            let env = [("YAVS_CYCLE_GUARD", "all")];
            let (cycle, err) = run_vars(&dir, &exact, "cycle_railguard", &env);
            let cycle = notes(cycle);
            let tail = |l: &[String]| -> Vec<String> {
                l.iter()
                    .map(|x| x[x.find("y=").expect("y")..].to_string())
                    .collect()
            };
            assert_eq!(tail(&cycle), tail(&normal), "{w}: {err}");
            assert!(
                err.contains("cycle mode: 2 of 4 processes run cycle-based")
                    && err.contains("second image: still followed when the run ended"),
                "{w}: {err}"
            );
        }
    }
}

/// A register the second image changes on a 'U' enable is given back its value in every word of
/// a register file of 80 words (`rail_keep`), as the normal mode keeps it.
#[test]
fn the_second_image_keeps_every_word_of_a_wide_register() {
    let dir = tempdir("cycle-railkeep");
    analyze(&dir, "railkeep.vhd", include_str!("cycle/railkeep.vhd"));
    for i in [3, 63, 64, 70, 79] {
        let g = format!("-gI={i}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let notes = |l: Vec<String>| -> Vec<String> {
                l.into_iter().filter(|x| x.contains("rf=")).collect()
            };
            let (normal, _) = run(&dir, &t, "cycle_railkeep");
            let normal = notes(normal);
            let want = format!("rf={:016X} rf0=0000000000000011", i * 4099 + 17);
            assert!(
                normal.len() == 1 && normal[0].ends_with(&want),
                "{normal:?}"
            );
            let exact = [&t[..], &["--cycle"]].concat();
            let (cycle, err) = run(&dir, &exact, "cycle_railkeep");
            let cycle = notes(cycle);
            assert!(
                cycle.len() == 1 && cycle[0].ends_with(&want),
                "{i}: {cycle:?}\n{err}"
            );
            assert!(
                err.contains("second image: still followed when the run ended"),
                "{i}: {err}"
            );
        }
    }
}

/// Combinational choices on a 'U' never show a 'U' the normal mode does not have: where the
/// region's image reads the 'U' as 0 and takes a branch the normal mode does not (`s = '0'`,
/// `unsigned(ad) < 2`), the member's outputs take the region's values on the second image too;
/// a 'U' that reaches an output without a choice still shows. Widths 1 to 128.
#[test]
fn combinational_choices_on_a_u_show_no_u_the_normal_mode_lacks() {
    let dir = tempdir("cycle-railcomb");
    analyze(&dir, "railcomb.vhd", include_str!("cycle/railcomb.vhd"));
    for w in [1, 63, 64, 65, 128] {
        let g = format!("-gW={w}");
        for t in tiers() {
            let t = [&t[..], &[&g[..]]].concat();
            let notes = |l: Vec<String>| -> Vec<String> {
                l.into_iter().filter(|x| x.contains(" yd=")).collect()
            };
            let (zero, u) = ("0".repeat(w), "U".repeat(w));
            let want = format!("y0={zero} y1={zero} yn={zero} yl={zero} yd={u}");
            let (normal, _) = run(&dir, &t, "cycle_railcomb");
            let normal = notes(normal);
            assert!(
                normal.len() == 1 && normal[0].ends_with(&want),
                "{normal:?}"
            );
            let exact = [&t[..], &["--cycle"]].concat();
            let (cycle, err) = run(&dir, &exact, "cycle_railcomb");
            let cycle = notes(cycle);
            assert!(
                cycle.len() == 1 && cycle[0].ends_with(&want),
                "{w}: {cycle:?}\n{err}"
            );
            assert!(
                err.contains("second image: still followed when the run ended"),
                "{w}: {err}"
            );
        }
    }
}

/// `YAVS_CYCLE_RAIL` takes `off`, `no-reseed` and `cap=N`; another value is an error naming
/// them, before the run.
#[test]
fn the_rail_knob_names_its_values() {
    let dir = tempdir("cycle-railknob");
    analyze(&dir, "rail.vhd", include_str!("cycle/rail.vhd"));
    let args = [
        "run",
        "--work=work:lib",
        "--no-cache",
        "--cycle",
        "--two-state",
        "cycle_rail",
    ];
    let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_RAIL", "of")]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success() && o.stdout.is_empty(), "{err}");
    assert!(err.contains("E4104") && err.contains("`off`"), "{err}");
    let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_RAIL", "no-reseed")]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// A scalar `out` variable parameter starts at its subtype's `'left` in cycle mode as in the
/// normal mode (#1184). A scalar `out` or `inout` actual that the body names (a procedure
/// declared in the process) or that an array `in` actual also names keeps the process in the
/// event kernel, since cycle mode binds it by reference and the LRM copies it (#1187), unless
/// the body assigns the `out` formal on every path and reaches the actual only before that.
#[test]
fn an_out_parameter_starts_at_its_default_in_cycle_mode() {
    let dir = tempdir("cycle-out-default");
    analyze(
        &dir,
        "outdef.vhd",
        "library ieee;\nuse ieee.std_logic_1164.all;\n\
         entity cycle_outdef is end entity;\narchitecture a of cycle_outdef is\n\
         \x20 procedure none (x : out natural) is begin end procedure;\n\
         \x20 procedure pk (x : out std_ulogic; a : in std_ulogic_vector) is\n  begin\n\
         \x20   if a(0) = '1' then x := '1'; end if;\n  end procedure;\n\
         \x20 procedure pk2 (x : out std_ulogic; a : in std_ulogic_vector) is\n  begin\n\
         \x20   if a(0) = '1' then x := '1'; else x := '0'; end if;\n  end procedure;\n\
         \x20 signal clk : std_ulogic := '0';\n  signal o1, o2, o4, o6, o7 : natural;\n  signal o3, o5 : std_ulogic;\nbegin\n\
         \x20 p : process (clk)\n    variable v : natural := 5;\n  begin\n\
         \x20   if rising_edge(clk) then\n      v := 5;\n      none(v);\n      o1 <= v;\n\
         \x20   end if;\n  end process;\n\
         \x20 q : process (clk)\n    variable w : natural := 5;\n\
         \x20   procedure seen (x : out natural) is\n    begin\n\
         \x20     if w = 5 then x := 1; end if;\n    end procedure;\n  begin\n\
         \x20   if rising_edge(clk) then\n      w := 5;\n      seen(w);\n      o2 <= w;\n\
         \x20   end if;\n  end process;\n\
         \x20 r : process (clk)\n    variable arr : std_ulogic_vector(0 to 1) := \"11\";\n  begin\n\
         \x20   if rising_edge(clk) then\n      arr := \"11\";\n      pk(arr(0), arr);\n\
         \x20     o3 <= arr(0);\n    end if;\n  end process;\n\
         \x20 u : process (clk)\n    variable z : natural := 5;\n\
         \x20   variable brr : std_ulogic_vector(0 to 1) := \"11\";\n\
         \x20   procedure both (x : out natural) is\n    begin\n\
         \x20     if z = 5 then x := 1; else x := 2; end if;\n    end procedure;\n  begin\n\
         \x20   if rising_edge(clk) then\n      z := 5;\n      both(z);\n      o4 <= z;\n\
         \x20     brr := \"11\";\n      pk2(brr(0), brr);\n      o5 <= brr(0);\n\
         \x20   end if;\n  end process;\n\
         \x20 s : process (clk)\n    variable m, seen_m : natural := 5;\n\
         \x20   procedure bump (x : inout natural) is\n    begin\n\
         \x20     x := x + 1;\n      seen_m := m;\n    end procedure;\n  begin\n\
         \x20   if rising_edge(clk) then\n      m := 5;\n      bump(m);\n      o6 <= seen_m;\n\
         \x20   end if;\n  end process;\n\
         \x20 t : process (clk)\n    variable n, seen_n : natural := 5;\n\
         \x20   procedure late (x : out natural) is\n    begin\n\
         \x20     x := 1;\n      seen_n := n;\n    end procedure;\n  begin\n\
         \x20   if rising_edge(clk) then\n      n := 5;\n      late(n);\n      o7 <= seen_n;\n\
         \x20   end if;\n  end process;\n\
         \x20 tb : process\n  begin\n\
         \x20   for i in 1 to 2 loop\n      clk <= '1';\n      wait for 5 ns;\n\
         \x20     clk <= '0';\n      wait for 5 ns;\n\
         \x20     report \"o1=\" & integer'image(o1) & \" o2=\" & integer'image(o2)\n\
         \x20       & \" o3=\" & std_ulogic'image(o3) & \" o4=\" & integer'image(o4)\n\
         \x20       & \" o5=\" & std_ulogic'image(o5) & \" o6=\" & integer'image(o6)\n\
         \x20       & \" o7=\" & integer'image(o7);\n\
         \x20   end loop;\n    wait;\n  end process;\nend architecture;\n",
    );
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_outdef");
        let (cycle, err) = run(
            &dir,
            &[&t[..], &["--cycle", "--two-state"]].concat(),
            "cycle_outdef",
        );
        assert_eq!(normal.len(), 2, "{t:?} {normal:?}");
        assert!(
            normal[1].ends_with("o1=0 o2=1 o3='1' o4=1 o5='1' o6=5 o7=5"),
            "{t:?} {normal:?}"
        );
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        assert!(
            err.contains("E4101 an `out` parameter X of `seen` whose actual the call also"),
            "{err}"
        );
        assert!(
            err.contains("E4101 an `out` parameter X of `pk` whose actual the call also"),
            "{err}"
        );
        assert!(
            err.contains("E4101 an `inout` parameter X of `bump` whose actual the call also"),
            "{err}"
        );
        assert!(
            err.contains("E4101 an `out` parameter X of `late` whose actual the call also"),
            "{err}"
        );
        assert!(!err.contains("`both`") && !err.contains("`pk2`"), "{err}");
    }
}

/// Scalar `out` and `inout` formals of procedures that members call are checked as in the
/// normal mode (#1181, #1175): a value driven through a signal formal against the formal's
/// subtype, then (a checked call) the actual's, a formal passed on narrowed to its subtype, a
/// procedure that declares a subprogram not against the actual; a signal actual never at the
/// call; a variable `inout` actual at the call and an `out` one when copied back, an element's
/// place fixed at the call (#1182). A formal or subtype range that reads a variable keeps the
/// caller out: its value is the one at the declaration. An enumeration value or index shows as
/// its literal (#1178). Scalars only, so no widths.
#[test]
fn parameter_ranges_are_checked_as_in_the_normal_mode() {
    let dir = tempdir("cycle-sigrange");
    analyze(&dir, "sigrange.vhd", include_str!("cycle/sigrange.vhd"));
    let act = "value 10 outside of INTEGER range 0 to 9 for actual of signal parameter O";
    for (top, msg, members) in [
        ("sr_sig", act, "1 of 2"),
        ("sr_sigconc", act, "1 of 2"),
        ("sr_sigelem", act, "1 of 2"),
        ("sr_sigfield", act, "1 of 2"),
        ("sr_sigclk", act, "1 of 3"),
        (
            "sr_sigf",
            "value -1 outside of NATURAL range 0 to 2147483647 for parameter O",
            "1 of 2",
        ),
        (
            "sr_sigenum",
            "value S2 outside of ST range S0 to S1 for actual of signal parameter O",
            "2 of 3",
        ),
        (
            "sr_fwd",
            "value 5 outside of INTEGER range 10 to 15 for actual of signal parameter O",
            "1 of 3",
        ),
        (
            "sr_varout",
            "value 10 outside of INTEGER range 0 to 9 for actual of parameter O",
            "1 of 2",
        ),
        (
            "sr_varenum",
            "value S2 outside of SUB range S0 to S1 for actual of parameter O",
            "1 of 2",
        ),
        (
            "sr_inout",
            "value -5 outside of NATURAL range 0 to 2147483647 for parameter O",
            "1 of 2",
        ),
        (
            "sr_enumvar",
            "value WHITE outside of RGB range RED to BLUE for variable V",
            "1 of 2",
        ),
        (
            "sr_enumix",
            "index WHITE outside of range RED to BLUE for t",
            "1 of 2",
        ),
        (
            "sr_frange",
            "value 7 outside of INTEGER range 0 to 5 for parameter X",
            "0 of 2",
        ),
        (
            "sr_subrange",
            "value 8 outside of T range 0 to 5 for variable Z",
            "0 of 2",
        ),
    ] {
        let msg = format!("error[E3009]: {msg}");
        let note = format!("at 1 ns, delta 1, in process {top}.c");
        let mut shown = Vec::new();
        for mode in MODES {
            let mut args = vec!["run", "--work=work:lib", "--no-cache", "--tier=interp"];
            args.extend(mode);
            args.push(top);
            let o = yavs(&dir, &args);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "{top} {mode:?}: {err}");
            // A region that is empty runs nothing cycle-based: the normal mode's message.
            let want = if mode.contains(&"--two-state") && members != "0 of 2" {
                bare(&msg)
            } else {
                msg.clone()
            };
            assert!(
                err.contains(&want) && err.contains(&note),
                "{top} {mode:?}: {err}"
            );
            shown.push(err[err.find("error[").expect("an error")..].to_string());
            if !mode.is_empty() {
                assert!(
                    err.contains(&format!("cycle mode: {members} processes run cycle-based")),
                    "{top}: {err}"
                );
            }
        }
        assert_eq!(shown[0], shown[2], "{top}");
    }
    // The reason of a range read where it was not declared.
    let report = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "sr_frange",
        String::new(),
    );
    assert!(report.contains("whose range reads a variable"), "{report}");
    let report = with_report(
        &dir,
        &["--cycle", "--two-state"],
        "sr_subrange",
        String::new(),
    );
    assert!(
        report.contains("the subtype `t`, whose range reads a variable"),
        "{report}"
    );
    // In range, or a check the normal mode does not make.
    for (top, last) in [
        ("sr_sigok", "s=4"),
        ("sr_sigin", "s=-1"),
        ("sr_fin", "s=-1"),
        ("sr_nested", "s=10"),
        ("sr_twice", "s=4"),
        ("sr_index", "s=70"),
    ] {
        for t in tiers() {
            let (normal, _) = run(&dir, &t, top);
            let (cycle, err) = run(&dir, &[&t[..], &["--cycle", "--two-state"]].concat(), top);
            assert!(
                normal.len() == 2 && normal[1].ends_with(last),
                "{top} {normal:?}"
            );
            assert_eq!(cycle, normal, "{top} {t:?}\n{err}");
            assert!(
                err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                "{top}: {err}"
            );
        }
    }
}

/// Each name a wave file records with its value at the end of each time step where it changed
/// (FST keeps no deltas; a value written again unchanged is dropped). With `u_as_0` a `'U'` reads
/// as `0`, as the two-state region gives it.
fn wave_steps(path: &Path, u_as_0: bool) -> std::collections::BTreeMap<String, Vec<(u64, String)>> {
    let mut w = wellen::simple::read(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let h = w.hierarchy();
    let vars: Vec<(String, wellen::SignalRef)> = (h.all_vars())
        .map(|r| (h[r].full_name(h), h[r].signal_ref()))
        .collect();
    let refs: Vec<_> = vars.iter().map(|v| v.1).collect();
    w.load_signals(&refs);
    let tt = w.time_table().to_vec();
    let mut out = std::collections::BTreeMap::new();
    for (name, r) in vars {
        let mut v: Vec<(u64, String)> = Vec::new();
        for (t, val) in w.get_signal(r).expect("loaded").iter_changes() {
            let mut s = val.to_bit_string().unwrap_or_else(|| val.to_string());
            if u_as_0 {
                s = s.replace(['u', 'U'], "0");
            }
            let t = tt[t as usize];
            if v.last().is_some_and(|l| l.0 == t) {
                v.pop();
            }
            if v.last().is_none_or(|l| l.1 != s) {
                v.push((t, s));
            }
        }
        out.insert(name, v);
    }
    out
}

/// `--wave` under `--cycle`: the signals only the region reads (registers, logic on them, logic on
/// an input that changes between edges, a record, an array, a memory, a register without a start
/// value) have the normal mode's value at every time step, at widths around a word and under both
/// contracts; the memory of integers, which the region cannot export, is left out with a warning.
/// The console, the members and, without `--wave`, the region's outputs are those of a run without.
#[test]
fn waves_of_signals_inside_the_region_follow_the_normal_mode_per_time_step() {
    let outputs = |err: &str| -> usize {
        (err.lines())
            .find_map(|l| {
                let mut w = l.strip_prefix("cycle stats: ")?.split(' ');
                let n = w.next()?.parse().ok()?;
                (w.next() == Some("outputs")).then_some(n)
            })
            .unwrap_or_else(|| panic!("no outputs line: {err}"))
    };
    for w in [1, 63, 64, 65, 128] {
        let dir = tempdir(&format!("cycle-waves-{w}"));
        analyze(
            &dir,
            "waves.vhd",
            &with_width(include_str!("cycle/waves.vhd"), w),
        );
        let wave = |flags: &[&str], file: &str, env: &[(&str, &str)]| {
            let path = dir.join(file).display().to_string();
            let wf = format!("--wave={path}");
            let mut args = vec!["run", "--work=work:lib", "--no-cache", wf.as_str()];
            args.extend(flags);
            args.push("cycle_waves");
            let o = yavs_env(&dir, &args, env);
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            assert!(o.status.success(), "{w} {flags:?}: {err}");
            (String::from_utf8_lossy(&o.stdout).into_owned(), err)
        };
        let (n_out, _) = wave(&[], "n.fst", &[]);
        let (_, _) = wave(&["--wave-include=cycle_waves.dut.mix"], "nn.fst", &[]);
        for exact in [false, true] {
            let mode: &[&str] = if exact {
                &["--cycle"]
            } else {
                &["--cycle", "--two-state"]
            };
            let stats = [("YAVS_CYCLE_STATS", "1")];
            let (c_out, c_err) = wave(mode, "c.fst", &stats);
            // The result line as in the normal mode (the notes of the asserts differ in their deltas
            // and at time 0, cycle mode's own matter); the whole console as without `--wave`.
            let done = |o: &str| o.lines().find(|l| l.contains("dout=")).map(String::from);
            assert_eq!(done(&c_out), done(&n_out), "{w} exact={exact}");
            assert!(
                c_out.contains("cnt=13") && c_out.contains("c(0) set"),
                "{c_out}"
            );
            assert!(
                c_err.contains("cycle mode: 8 of 9 processes run cycle-based"),
                "{w} exact={exact}: {c_err}"
            );
            assert!(
                c_err.contains("warning[W0802]: `cycle_waves.dut.iram` is not recorded"),
                "{c_err}"
            );
            // Without `--wave`: the same console, members and outputs.
            let mut args = vec!["run", "--work=work:lib", "--no-cache"];
            args.extend(mode);
            args.push("cycle_waves");
            let o = yavs_env(&dir, &args, &stats);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(String::from_utf8_lossy(&o.stdout), c_out);
            assert!(err.contains("cycle mode: 8 of 9 processes"), "{err}");
            let (none, full) = (outputs(&err), outputs(&c_err));
            // Per time step, a 'U' as 0 under two-state values.
            let n = wave_steps(&dir.join("n.fst"), !exact);
            let c = wave_steps(&dir.join("c.fst"), false);
            // Four threads and each tier of the build record the same.
            if w == 64 {
                for t in tiers() {
                    let (out, _) = wave(&[mode, &t[..]].concat(), "ct.fst", &[]);
                    assert_eq!(out, c_out, "{t:?}");
                    assert_eq!(wave_steps(&dir.join("ct.fst"), false), c, "{t:?}");
                }
            }
            let inside = [
                "cycle_waves.dut.q",
                "cycle_waves.dut.mix2",
                "cycle_waves.dut.noinit",
            ];
            for k in inside {
                let k = n.keys().find(|n| n.starts_with(k)).expect("recorded");
                assert!(n[k].len() > 3, "{k} changes: {:?}", n[k]);
            }
            let kept: Vec<&String> = n.keys().filter(|k| !k.contains(".iram")).collect();
            assert!(n.len() > kept.len());
            assert_eq!(c.keys().collect::<Vec<_>>(), kept, "{w} exact={exact}");
            for k in kept {
                assert_eq!(c[k], n[k], "{w} exact={exact} {k}");
            }
            if exact {
                let k = n.keys().find(|n| n.contains("noinit")).unwrap();
                assert!(n[k][0].1.contains(['u', 'U']), "{:?}", n[k]);
            }
            // Only `mix`: the same as the normal mode's, and fewer outputs than the whole wave.
            let (_, err) = wave(
                &[mode, &["--wave-include=cycle_waves.dut.mix"]].concat(),
                "cn.fst",
                &stats,
            );
            let narrow = outputs(&err);
            assert!(none < narrow && narrow < full, "{none} {narrow} {full}");
            let (nn, cn) = (
                wave_steps(&dir.join("nn.fst"), !exact),
                wave_steps(&dir.join("cn.fst"), false),
            );
            assert!(!nn.is_empty());
            assert_eq!(cn, nn, "{w} exact={exact}");
        }
    }
}

/// The wave options are in the key of the cached analysis: a region cached without `--wave` is not
/// the one a run with it uses (its inside would stay at the start values), and the reverse.
#[test]
fn waves_under_cycle_do_not_take_a_region_cached_without_them() {
    let dir = tempdir("cycle-waves-cache");
    analyze(
        &dir,
        "waves.vhd",
        &with_width(include_str!("cycle/waves.vhd"), 8),
    );
    let run = |wave: bool| {
        let mut args = vec!["run", "--work=work:lib", "--cycle", "--two-state"];
        if wave {
            args.push("--wave=c.fst");
        }
        args.push("cycle_waves");
        let o = yavs_env(&dir, &args, &[("YAVS_CYCLE_STATS", "1")]);
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        assert!(o.status.success(), "{err}");
        (err.lines())
            .find(|l| l.starts_with("cycle stats: ") && l.contains(" outputs ("))
            .expect("an outputs line")
            .to_string()
    };
    let changes = || {
        let c = wave_steps(&dir.join("c.fst"), false);
        let k = c
            .keys()
            .find(|k| k.starts_with("cycle_waves.dut.q"))
            .unwrap();
        c[k].len()
    };
    let cold = run(false);
    let waved = run(true);
    assert!(changes() > 3);
    assert_ne!(cold, waved);
    // Warm runs, each after the other kind: the region of its own options.
    assert_eq!(run(false), cold);
    assert_eq!(run(true), waved);
    assert!(changes() > 3);
    std::fs::remove_dir_all(dir.join("lib/yavs-cache")).unwrap();
    assert_eq!(run(true), waved);
    assert_eq!(run(false), cold);
}
