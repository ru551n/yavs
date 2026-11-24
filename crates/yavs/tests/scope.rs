//! The scopes of `--two-state[=PATH]` and `--cycle[=PATH]` (A64): paths by name (a generate's
//! every iteration or one), the design under test found by default, one architecture lowered in
//! both variants, the errors of a path that names nothing and of scopes that overlap in part,
//! and `--cycle-report` as a modifier.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::tempdir;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .env("YAVS_TWO_CHECK", "1")
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn setup(name: &str) -> common::Tmp {
    let dir = tempdir(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cycle/scope.vhd");
    let o = yavs(
        &dir,
        &[
            "analyze",
            "--std=2008",
            "--work=work:lib",
            src.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    dir
}

/// The two report lines of a run (`q=... zq=...` and what the bus model saw), and stderr.
fn run(dir: &Path, flags: &[&str]) -> (String, String) {
    let mut args = vec!["run", "--work=work:lib"];
    args.extend(flags);
    args.push("scope_tb");
    let o = yavs(dir, &args);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(o.status.success(), "{flags:?}: {err}");
    let out = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = (out.lines())
        .filter_map(|l| l.rsplit_once(": ").map(|x| x.1))
        .filter(|l| l.starts_with("q=") || l.starts_with("vc sees"))
        .collect();
    (lines.join(" | "), err)
}

/// What a run prints with the cells of `two` (the bits of `q(3 downto 0)`: generate cells 1 and
/// 0, the bus model's, the design's) and the bus model in the two-state scope.
fn want(cells: [bool; 4], vc: bool) -> String {
    let q: String = cells.iter().map(|&t| if t { '0' } else { 'U' }).collect();
    let zq: String = cells.iter().map(|&t| if t { '0' } else { 'X' }).collect();
    format!("vc sees '{}' | q={q} zq={zq}", if vc { '0' } else { '1' })
}

fn tiers() -> Vec<Vec<&'static str>> {
    let mut t = vec![
        vec!["--tier=interp"],
        vec!["--tier=interp", "--threads", "4"],
    ];
    if cfg!(feature = "clift") {
        t.push(vec!["--tier=clift"]);
    }
    t
}

#[test]
fn a_path_names_the_two_state_scope() {
    let dir = setup("scope-paths");
    // Generate cell 1, cell 0, the bus model's cell, the design's cell.
    let cases: [(&[&str], [bool; 4], bool); 7] = [
        (&[], [false; 4], false),
        (
            &["--two-state=scope_tb.dut"],
            [false, false, false, true],
            false,
        ),
        (
            &["--two-state=scope_tb.g"],
            [true, true, false, false],
            false,
        ),
        (
            &["--two-state=SCOPE_TB.G(1)"],
            [true, false, false, false],
            false,
        ),
        (
            &["--two-state=scope_tb.vc"],
            [false, false, true, false],
            true,
        ),
        (
            &["--two-state=scope_tb.vc", "--two-state=scope_tb.g(0)"],
            [false, true, true, false],
            true,
        ),
        (&["--two-state=scope_tb"], [true; 4], true),
    ];
    for (flags, cells, vc) in cases {
        for t in tiers() {
            for cache in ["--no-cache", ""] {
                let mut f = flags.to_vec();
                f.extend(&t);
                f.extend([cache].iter().filter(|c| !c.is_empty()));
                let (got, err) = run(&dir, &f);
                assert_eq!(got, want(cells, vc), "{f:?}: {err}");
                assert!(!err.contains("design under test"), "{f:?}: {err}");
            }
        }
    }
}

/// Without PATH the scope is the design under test: the instances with RTL and (nearly) no
/// testbench processes, here the design and the generate's cells; the bus model, whose process
/// waits for a time, stays nine-valued and sees its `'Z'`. The same run as with those paths.
#[test]
fn the_design_under_test_is_the_default_scope() {
    let dir = setup("scope-auto");
    let paths = [
        "--two-state=scope_tb.dut",
        "--two-state=scope_tb.g(0).cell",
        "--two-state=scope_tb.g(1).cell",
    ];
    for t in tiers() {
        let mut f = vec!["--two-state"];
        f.extend(&t);
        let (got, err) = run(&dir, &f);
        assert_eq!(got, want([true, true, false, true], false), "{err}");
        assert!(
            err.contains(
                "design under test: scope_tb.dut, scope_tb.g(0).cell, scope_tb.g(1).cell (the \
                 scope of --two-state; an option's PATH chooses another)"
            ),
            "{err}"
        );
        let mut f = paths.to_vec();
        f.extend(&t);
        assert_eq!(run(&dir, &f).0, got);
    }
    // Plain `--cycle` takes the whole design (A76): no design under test is looked for.
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle",
            "--cycle-report",
            "scope_tb",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        o.status.success()
            && err.contains("  cycle scope: the whole design")
            && !err.contains("design under test")
            && !err.contains("outside the cycle scope"),
        "{err}"
    );
    // Cycle mode finds the same design: the bus model's cell stays in the event kernel.
    let (got, err) = run(&dir, &["--cycle", "--two-state", "--tier=interp"]);
    assert_eq!(got, want([true, true, false, true], false), "{err}");
    assert!(
        err.contains("the scope of --cycle and --two-state"),
        "{err}"
    );
}

#[test]
fn the_cycle_report_shows_the_scopes() {
    let dir = setup("scope-report");
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle=scope_tb.g(0)",
            "--cycle-report",
            "--two-state",
            "scope_tb",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{err}");
    assert!(err.contains("  cycle scope: scope_tb.g(0)\n"), "{err}");
    assert!(
        err.contains(
            "  two-state scope, the design under test found: scope_tb.dut, scope_tb.g(0).cell, \
             scope_tb.g(1).cell\n"
        ),
        "{err}"
    );
    // The other cells fit, outside the cycle scope; the testbench's process is behavioural.
    assert!(
        err.contains("    outside the cycle scope (6 processes):\n"),
        "{err}"
    );
    assert!(
        err.lines()
            .any(|l| l.contains("scope_tb.dut.c.process (scope.vhd:")
                && l.ends_with("it could run cycle-based, but is not in the cycle scope")),
        "{err}"
    );
    assert!(err.contains("cycle mode: 1 of 15 processes"), "{err}");
    assert!(
        err.lines().any(
            |l| l.starts_with("    scope_tb.g(0).cell.process (scope.vhd:29)")
                && l.ends_with("clocked")
        ),
        "{err}"
    );
}

#[test]
fn a_path_that_names_nothing_is_an_error() {
    let dir = setup("scope-unknown");
    for (flag, help) in [
        (
            "--two-state=scope_tb.dutt",
            "a path with a similar name exists: `scope_tb.dut`",
        ),
        (
            "--two-state=dut",
            "a path starts at the top-level unit: `--two-state=scope_tb.dut`",
        ),
        (
            "--cycle=scope_tb.gg",
            "a path with a similar name exists: `scope_tb.g`",
        ),
        (
            "--two-state=nothing.here",
            "`yavs elab --print-hierarchy TOP` lists the paths",
        ),
    ] {
        let o = yavs(&dir, &["run", "--work=work:lib", flag, "scope_tb"]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{flag}: {err}");
        assert!(err.contains("error[E2122]"), "{flag}: {err}");
        assert!(err.contains(help), "{flag}: {err}");
    }
}

#[test]
fn scopes_that_overlap_in_part_are_an_error() {
    let dir = setup("scope-overlap");
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle=scope_tb",
            "--two-state=scope_tb.dut",
            "scope_tb",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("error[E2123]"), "{err}");
    // Inside the two-state scope, or apart from it, is fine: a region apart from it keeps the
    // metavalues (the bus model's cell shows its 'U' and sees the 'Z').
    for (f, cells, vc) in [
        (
            ["--cycle=scope_tb.dut", "--two-state=scope_tb"],
            [true; 4],
            true,
        ),
        (
            ["--cycle=scope_tb.vc", "--two-state=scope_tb.dut"],
            [false, false, false, true],
            false,
        ),
    ] {
        let (got, err) = run(&dir, &[f[0], f[1], "--tier=interp"]);
        assert_eq!(got, want(cells, vc), "{f:?}: {err}");
    }
}

/// A flat design is its own design under test, without its testbench-kind processes (A76): its
/// stimulus process stays nine-valued and reads the 'Z' line as 'Z'; a strict run on a design
/// with no RTL is an error.
#[test]
fn a_flat_design_keeps_its_testbench_processes_out() {
    let dir = tempdir("scope-flat");
    let src = dir.join("flat.vhd");
    std::fs::write(
        &src,
        "library ieee; use ieee.std_logic_1164.all;
entity flat is end;
architecture a of flat is
  signal clk : std_logic := '0';
  signal line : std_logic := 'Z';
  signal q, r : std_logic;
begin
  clk <= not clk after 5 ns when now < 30 ns else clk;
  q <= '1' when line = 'Z' else '0';
  process (clk) begin if rising_edge(clk) then r <= q; end if; end process;
  process begin
    wait for 20 ns;
    report \"seen=\" & std_logic'image(line) & \" q=\" & std_logic'image(q) & \" r=\" & std_logic'image(r);
    wait;
  end process;
end;
-- A process of the testbench calls its own procedure with a narrower actual (a checked call):
-- the testbench's variant of the architecture has its own checked body.
entity flatcall is end;
architecture a of flatcall is
  subtype digit is integer range 0 to 9;
  signal s, c : digit := 0;
  procedure drv (signal o : out integer; x : integer) is begin o <= x; end procedure;
begin
  drv(c, s);
  process
    procedure local (signal o : out integer) is begin o <= 3; end procedure;
  begin
    wait for 1 ns;
    local(s);
    wait for 1 ns;
    report \"s=\" & integer'image(s) & \" c=\" & integer'image(c);
    wait;
  end process;
end;
-- A delayed assignment of logic is RTL, so it is a design under test; a register of it without an
-- initial value starts at 0 in the two-state scope.
library ieee; use ieee.std_logic_1164.all;
entity delay_dut is port (d : in std_logic; q : out std_logic); end;
architecture a of delay_dut is
  signal r : std_logic;
begin
  q <= d after 1 ns;
  process (d) begin if rising_edge(d) then r <= not r; end if; end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity delay_tb is end;
architecture a of delay_tb is
  signal d, q : std_logic := '0';
begin
  dut : entity work.delay_dut port map (d, q);
  process begin
    wait for 1 ns;
    report \"r=\" & std_logic'image(<<signal .delay_tb.dut.r : std_logic>>);
    wait;
  end process;
end;
-- A pad model in the design under test: its drivers of `'Z'` stay out of the two-state scope
-- found (A78), so the testbench's values reach the line as in the normal mode (two-state, an
-- element driven with `'Z'` in a generate drove `0` against them).
library ieee; use ieee.std_logic_1164.all;
entity pad_dut is
  port (o, oe : in std_ulogic_vector(3 downto 0); i : out std_ulogic_vector(3 downto 0);
        pad : inout std_logic_vector(3 downto 0));
end;
architecture a of pad_dut is
begin
  g : for k in 0 to 3 generate
    pad(k) <= o(k) when oe(k) = '1' else 'Z';
  end generate;
  i <= to_x01(pad);
end;
library ieee; use ieee.std_logic_1164.all;
entity pad_tb is end;
architecture a of pad_tb is
  signal o, oe, i : std_ulogic_vector(3 downto 0) := \"0000\";
  signal pad : std_logic_vector(3 downto 0);
begin
  dut : entity work.pad_dut port map (o, oe, i, pad);
  process begin
    pad <= \"0101\";
    wait for 1 ns;
    report \"pad=\" & to_string(pad) & \" i=\" & to_string(i);
    wait;
  end process;
end;
entity bare is end;
architecture a of bare is
begin
  process begin report \"hello\"; wait for 1 ns; wait; end process;
end;
",
    )
    .unwrap();
    let o = yavs(
        &dir,
        &[
            "analyze",
            "--std=2008",
            "--work=work:lib",
            src.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for flags in [&["--two-state"][..], &["--cycle", "--two-state"]] {
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("flat");
        let o = yavs(&dir, &args);
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(o.status.success(), "{err}");
        // The design's `q` reads the line as 0; the stimulus sees the 'Z'.
        assert!(
            out.contains("seen='Z' q='0' r='0'"),
            "{flags:?}: {out}{err}"
        );
        assert!(
            err.contains("design under test: flat (the scope of"),
            "{err}"
        );
    }
    for flags in [&["--two-state"][..], &["--cycle", "--two-state"]] {
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("flatcall");
        let o = yavs(&dir, &args);
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(
            o.status.success() && out.contains("s=3 c=3"),
            "{flags:?}: {out}{err}"
        );
    }
    for (flags, r) in [(&["--two-state"][..], "r='0'"), (&[], "r='U'")] {
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("delay_tb");
        let o = yavs(&dir, &args);
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(
            o.status.success() && out.contains(r),
            "{flags:?}: {out}{err}"
        );
        if !flags.is_empty() {
            assert!(
                err.contains("design under test: delay_tb.dut (the scope of"),
                "{err}"
            );
        }
    }
    for flags in [&[][..], &["--two-state"], &["--cycle", "--two-state"]] {
        let want = "pad=0101 i=0101";
        let mut args = vec!["run", "--work=work:lib"];
        args.extend(flags);
        args.push("pad_tb");
        let o = yavs(&dir, &args);
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(
            o.status.success() && out.contains(want),
            "{flags:?}: {out}{err}"
        );
        if !flags.is_empty() {
            assert!(
                err.contains("design under test: pad_tb.dut (the scope of"),
                "{err}"
            );
        }
    }
    // A strict run whose scope is the design under test refuses a design without one; plain
    // `--cycle-strict` takes the whole design (A76), whose top-level unit is never an error.
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle-strict",
            "--two-state",
            "bare",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("error[E4106]"), "{err}");
    let o = yavs(&dir, &["run", "--work=work:lib", "--cycle-strict", "bare"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        o.status.success() && !err.contains("design under test"),
        "{err}"
    );
}

#[test]
fn the_cycle_report_needs_a_cycle_option() {
    let dir = setup("scope-alone");
    let o = yavs(
        &dir,
        &["run", "--work=work:lib", "--cycle-report", "scope_tb"],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("error[E4105]"), "{err}");
    // With `--cycle-strict` its errors come first (the cells' `q` is one signal split between the
    // design under test, the scope with `--two-state`, and the bus model).
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--cycle-strict",
            "--cycle-report",
            "--two-state",
            "scope_tb",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("error[E4134]") && !err.contains("E4105"),
        "{err}"
    );
}
