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
            err.contains("wake, 1 wake the region (cycle_partial.din)"),
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
    assert!(counts[0].1.starts_with("E4122 "), "{counts:?}");
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
  signal lit : std_ulogic_vector(3 downto 0);
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= din;
      lit <= \"X1U0\";
    end if;
  end process;
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
        // At initialisation: 5 in `din`, 9 in `q` and 4 in `lit` (no initial value); `din`
        // never changes, so it is not read again.
        assert!(
            err.contains(
                "cycle mode: 18 metavalue elements read as 0 (18 at initialisation, 0 at changes \
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
    ] {
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
            if mode.is_some() {
                assert!(
                    err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                    "{err}"
                );
            }
        }
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
        // A part of an element at the index just before the array.
        (
            "cycle_eslice",
            "error[E3009]: index -1 outside of range 0 to 3 for v",
            "at 30 ns, delta 1, in process cycle_eslice.reg",
        ),
    ] {
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
            if mode.is_some() {
                assert!(
                    err.contains("cycle mode: 1 of 2 processes run cycle-based"),
                    "{err}"
                );
            }
        }
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

/// Slices whose bounds come from variables: known values fold (a loop parameter through two
/// variables, both branches, a `case`, an `inout` formal, a function's locals), unknown ones
/// do not (branches that differ, a loop that may exit, a state variable), and two variables
/// chosen at run time keep their process out instead of being taken for a one-element slice,
/// which failed at run time with a false length error.
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
                err.contains("cycle mode: 5 of 7 processes run cycle-based"),
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

/// Arrays of integers and enumerations in variables and constants; the processes that drive
/// signals of them stay out with the reason.
#[test]
fn arrays_of_integers_give_the_same_result() {
    let dir = tempdir("cycle-intarr");
    analyze(&dir, "intarr.vhd", include_str!("cycle/intarr.vhd"));
    for t in tiers() {
        let (normal, _) = run(&dir, &t, "cycle_intarr");
        let (cycle, err) = run(&dir, &[&t[..], &["--cycle"]].concat(), "cycle_intarr");
        assert_eq!(normal.len(), 16, "{t:?} {normal:?}");
        assert_eq!(cycle, normal, "{t:?}\n{err}");
        // `reg` and `cmb` join; `lanes` and `st` drive arrays of integers and enumerations.
        assert!(
            err.contains("cycle mode: 2 of 8 processes run cycle-based"),
            "{err}"
        );
        assert!(
            err.contains("an array signal of integers or enumerations is not supported in cycle mode yet (signal cycle_intarr.sh)"),
            "{err}"
        );
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
    let designs: [(&str, String, &str, &[&str], &str); 6] = [
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
        (
            "rtl_ram",
            include_str!("../../../tools/bench/designs/rtl_ram.vhd"),
            "bench_rtl_ram",
            "(a RAM) is not supported in cycle mode yet",
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
        // A function the evaluation gives up on, a loop past the budget: no pruning.
        ("give_up", "E4119", "if half_known(3)"),
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
        report.contains("subset: 27 of 81 processes fit, 1 loop group among them")
            && report.contains(
                "loop group of 1 process: 3 passes, or 3 pieces after a split: sub_tb.dut.levels"
            ),
        "{report}"
    );
    // Asserts and reports are counted, never a reason to stay out.
    assert!(
        report.contains("2 asserts and reports not evaluated in cycle mode, held by 2 cycle-based"),
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
                "cycle mode: 12 of 19 processes run cycle-based, on the rising edge of \
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
            "cycle_clocks.p4",
            "cycle_clocks.pg",
            "cycle_clocks.pgs",
            "cycle_clocks.pl",
            "cycle_clocks.pr",
        ];
        for (name, section) in sections(&err, &names) {
            assert_eq!(section, "cycle-based:", "{name}\n{err}");
        }
        // The logic between c1 and the clocks whose edges may come in another delta stays in the
        // kernel (with it in the region, `gs` differs).
        for name in ["cycle_clocks.t <= ...", "cycle_clocks.u <= ..."] {
            let (section, line) = row(&err, name);
            assert_eq!(
                section, "in the subset, not implemented in cycle mode yet:",
                "{err}"
            );
            assert!(line.contains("may come in a later delta"), "{line}");
        }
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
/// edge and at time 0, a wide input changed in one word, an output read and driven back outside.
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
        // times and `r.b` as it started.
        assert!(normal[2].contains("i=3 qs=A5 "), "{normal:?}");
        assert!(
            normal[3].contains(
                "i=4 qs=00 qw=000000E000000000 qn=21 qe=0 qz=1 qf=1 qm=XX01 y=0100 wm=0000001E \
                 qr=11"
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
