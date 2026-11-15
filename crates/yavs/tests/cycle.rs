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
        // At initialisation: 5 in `din`, 9 in `q` and 4 in `lit` (no initial value); then the 5
        // of `din` at each of the 2 edges.
        assert!(
            err.contains(
                "cycle mode: 28 metavalue elements read as 0 (18 at initialisation, 10 at the \
                 region's inputs)"
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
