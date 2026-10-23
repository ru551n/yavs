//! Natives, concatenations, layouts and signal reads in compiled code (`--tier=llvm`, a build with
//! the `llvm` feature) against the interpreter: output and exit status are the same, also when a
//! native ends the simulation (a warning of `numeric_std` at the stop severity).
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
begin
  process
    variable u : unsigned(3 downto 0) := "1011";
    variable m : unsigned(3 downto 0) := "10X1";
    variable n : integer := 0;
    variable s : string(1 to 3);
  begin
    for i in 0 to 5 loop
      n := n + to_integer(u + i);
      report "i=" & integer'image(i) & " n=" & integer'image(n) & " u=" & integer'image(to_integer(u));
    end loop;
    s := "a" & character'image('b')(2) & "c";
    report s & " " & natural'image(s'length);
    n := to_integer(m);
    report "after the warning " & integer'image(n);
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

fn run(dir: &Path, tier: &str, severity: &str) -> (Option<i32>, String) {
    let o = yavs(
        dir,
        &[
            "run",
            "--work=work:work",
            &format!("--tier={tier}"),
            &format!("--exit-severity={severity}"),
            "t",
        ],
    );
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn natives_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_natives");
    std::fs::write(dir.join("t.vhd"), SRC).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "t.vhd"]);
    assert!(a.status.success(), "{a:?}");
    for severity in ["failure", "warning"] {
        let want = run(&dir, "interp", severity);
        assert!(want.1.contains("i=5 n="), "{want:?}");
        // The warning is the last thing that happens at the stop severity.
        assert_eq!(
            want.1.contains("after the warning"),
            severity == "failure",
            "{want:?}"
        );
        for tier in ["llvm", "auto"] {
            assert_eq!(run(&dir, tier, severity), want, "{tier} at {severity}");
        }
    }
}

/// Vectors of every length class read from signals in a clocked process, among them the ones
/// whose last group of eight codes is partly past the signal (1, 4, 9, 12 elements) and the
/// ones that fill their plane block (64) or need two (65).
const SIGNALS: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity s is end;
architecture a of s is
  signal clk : std_logic := '0';
  signal v1 : std_logic_vector(0 downto 0) := "1";
  signal v4 : std_logic_vector(3 downto 0) := "10X1";
  signal v9 : unsigned(8 downto 0) := (others => '0');
  signal v12 : std_logic_vector(11 downto 0) := (others => 'Z');
  signal v33 : unsigned(32 downto 0) := (others => '0');
  signal v64 : unsigned(63 downto 0) := (others => '0');
  signal v65 : unsigned(64 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 80 ns else clk;
  process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      v1 <= std_logic_vector(to_unsigned(n, 1));
      v4 <= std_logic_vector(to_unsigned(n, 4));
      v9 <= v9 + 37;
      v12 <= std_logic_vector(to_unsigned(n * 111, 12));
      v33 <= v33 + 1000003;
      v64 <= (v64 sll 3) + to_unsigned(n * 1000003, 64);
      v65 <= (v65 sll 5) + to_unsigned(n * 7919, 65);
      report std_logic'image(v1(0)) & " " & to_string(v4) & " " & to_string(v9) & " " &
             to_string(v12) & " " & to_string(v33) & " " & to_string(v64) & " " & to_string(v65);
    end if;
  end process;
end;
"#;

#[test]
fn signal_reads_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_signals");
    std::fs::write(dir.join("s.vhd"), SIGNALS).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "s.vhd"]);
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &str| {
        let o = yavs(
            &dir,
            &[
                "run",
                "--work=work:work",
                &format!("--tier={tier}"),
                "--stop-time=100ns",
                "s",
            ],
        );
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let want = go("interp");
    assert!(want.1.contains("** Note"), "{want:?}");
    for tier in ["llvm", "auto"] {
        assert_eq!(go(tier), want, "{tier}");
    }
}

/// Scalar signals of enumeration, integer, boolean, real and time elements, and logic vectors
/// wider than 64 elements, read in a clocked process: compiled code reads all of them from the
/// kernel's bytes and plane blocks.
const KINDS: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity k is end;
architecture a of k is
  type st_t is (idle, run, done, fail);
  signal clk : std_logic := '0';
  signal st : st_t := idle;
  signal cnt : integer := -5;
  signal flag : boolean := false;
  signal r : real := 1.5;
  signal w128 : std_logic_vector(127 downto 0) := (others => '0');
  signal w200 : unsigned(199 downto 0) := (others => '0');
  signal t : time := 3 ns;
  signal small : natural range 0 to 200 := 7;
begin
  clk <= not clk after 5 ns when now < 200 ns else clk;
  process (clk)
    variable n : integer := 0;
    variable acc : integer := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      case st is
        when idle => st <= run;
        when run => if cnt > 0 then st <= done; end if;
        when done => st <= fail;
        when fail => st <= idle;
      end case;
      cnt <= cnt + 2;
      flag <= not flag;
      if flag then r <= r * 2.0; end if;
      if cnt < 0 then acc := acc + 1; end if;
      w128 <= w128(126 downto 0) & clk;
      w200 <= w200 + to_unsigned(n, 200);
      t <= t + 1 ns;
      small <= (small * 3) mod 201;
      report st_t'image(st) & " " & integer'image(cnt) & " " & boolean'image(flag) & " " &
             real'image(r) & " " & to_hstring(w128) & " " & to_hstring(std_logic_vector(w200)) &
             " " & time'image(t) & " " & integer'image(small) & " " & integer'image(acc);
    end if;
  end process;
end;
"#;

#[test]
fn scalar_and_wide_signal_reads_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_kinds");
    std::fs::write(dir.join("k.vhd"), KINDS).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "k.vhd"]);
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["run", "--work=work:work", &format!("--tier={tier}"), "k"])
            .env("YAVS_COUNTERS", "kinds")
            .current_dir(&dir)
            .output()
            .expect("run yavs");
        let census: Vec<String> = String::from_utf8_lossy(&o.stderr)
            .lines()
            .filter(|l| l.starts_with("counters: cb_") && l.contains('.'))
            .map(str::to_string)
            .collect();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            census,
        )
    };
    let want = go("interp");
    assert!(want.1.matches("** Note").count() >= 20, "{want:?}");
    for tier in ["llvm", "auto"] {
        let got = go(tier);
        assert_eq!((got.0, &got.1), (want.0, &want.1), "{tier}");
        // The reads of all the kinds are answered in compiled code.
        if tier == "llvm" {
            assert!(
                !got.2
                    .iter()
                    .any(|l| l.contains(".SigRead=") || l.contains(".SigReadScalar=")),
                "{:?}",
                got.2
            );
        }
    }
}

/// The measuring aids of the compiled tier: `YAVS_LLVM_KINDS` prints the LLVM instructions per
/// IR kind, and `YAVS_LLVM_ABLATE` has the interpreter step a kind, which then shows in the
/// host-call counters, with the same output.
#[test]
fn llvm_measuring_aids() {
    let dir = tempdir("compiled_aids");
    std::fs::write(dir.join("k.vhd"), KINDS).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "k.vhd"]);
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &str, env: &[(&str, &str)]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["run", "--work=work:work", &format!("--tier={tier}"), "k"])
            .env("YAVS_COUNTERS", "kinds")
            .envs(env.iter().copied())
            .current_dir(&dir)
            .output()
            .expect("run yavs")
    };
    let want = go("interp", &[]);
    let kinds = go("llvm", &[("YAVS_LLVM_KINDS", "1")]);
    let err = String::from_utf8_lossy(&kinds.stderr);
    assert!(err.contains("kinds: total llvm insts"), "{err}");
    assert!(err.contains("kinds: SigReadScalar"), "{err}");
    assert_eq!(kinds.stdout, want.stdout);
    let ablated = go("llvm", &[("YAVS_LLVM_ABLATE", "SigReadScalar")]);
    assert_eq!(ablated.stdout, want.stdout);
    let err = String::from_utf8_lossy(&ablated.stderr);
    assert!(err.contains(".SigReadScalar="), "{err}");
}

/// Equality of logic vectors of every length class, with metavalues in them.
const COMPARES: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity q is end;
architecture a of q is
  signal clk : std_logic := '0';
  signal a1, b1 : std_logic_vector(0 downto 0) := "1";
  signal a5, b5 : std_logic_vector(4 downto 0) := "10X1Z";
  signal a64, b64 : std_logic_vector(63 downto 0) := (others => '1');
  signal a65, b65 : std_logic_vector(64 downto 0) := (others => '0');
  signal a100, b100 : std_logic_vector(99 downto 0) := (others => 'U');
begin
  clk <= not clk after 5 ns when now < 120 ns else clk;
  process (clk)
    variable n : integer := 0;
    variable s : string(1 to 10);
    procedure mark (k : positive; v : boolean) is
    begin
      if v then s(k) := '1'; else s(k) := '0'; end if;
    end;
  begin
    if rising_edge(clk) then
      n := n + 1;
      mark(1, a1 = b1);
      mark(2, a5 = b5);
      mark(3, a64 = b64);
      mark(4, a65 = b65);
      mark(5, a100 = b100);
      mark(6, a5(3 downto 1) = b5(3 downto 1));
      mark(7, a64(62 downto 0) = b64(62 downto 0));
      mark(8, a64(63 downto 1) /= b64(63 downto 1));
      mark(9, a100(70 downto 3) = b100(70 downto 3));
      mark(10, a5 /= "10X1Z");
      report s;
      a1(0) <= to_unsigned(n, 1)(0);
      b1(0) <= to_unsigned(n / 2, 1)(0);
      a5 <= std_logic_vector(to_unsigned(n, 5));
      b5 <= std_logic_vector(to_unsigned(n / 3, 5)) when n mod 4 /= 0 else a5;
      b64 <= (others => to_unsigned(n, 1)(0)) when n mod 3 = 0 else a64;
      a64 <= (others => to_unsigned(n / 2, 1)(0)) when n mod 5 /= 0 else (others => 'X');
      a65(n mod 65) <= '1';
      b65(n mod 7) <= '1';
      a100 <= (others => '0');
      b100 <= (others => '0') when n mod 2 = 0 else (others => 'H');
      -- Runs set to one code: inside a block, across two, a whole one.
      if n mod 3 = 0 then
        a65(10 downto 2) <= (others => 'H');
        a100(99 downto 30) <= (others => '-');
        b64 <= (others => 'Z');
      elsif n mod 3 = 1 then
        a5(2 downto 1) <= (others => 'L');
        b100(64 downto 63) <= (others => 'X');
        a64(63 downto 0) <= (others => 'U');
      end if;
    end if;
  end process;
end;
"#;

#[test]
fn logic_equality_in_compiled_code_matches_the_interpreter() {
    let dir = tempdir("compiled_compares");
    std::fs::write(dir.join("q.vhd"), COMPARES).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "q.vhd"]);
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &str| {
        let o = yavs(
            &dir,
            &["run", "--work=work:work", &format!("--tier={tier}"), "q"],
        );
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let want = go("interp");
    assert!(want.1.matches("** Note").count() >= 10, "{want:?}");
    for tier in ["llvm", "auto"] {
        assert_eq!(go(tier), want, "{tier}");
    }
}

/// Two variables of one record type with different constraints in one process: the layout a
/// process's frame keeps for the type is one of them, the other is made each time.
const RECORDS: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity lc is end;
architecture a of lc is
  type rec_t is record
    d : std_logic_vector;
    n : integer;
    e : std_logic_vector;
  end record;
  subtype r8 is rec_t(d(7 downto 0), e(3 downto 0));
  subtype r16 is rec_t(d(15 downto 0), e(7 downto 0));
  signal clk : std_logic := '0';
  signal q8  : std_logic_vector(7 downto 0) := (others => '0');
  signal q16 : std_logic_vector(15 downto 0) := (others => '0');
  signal qe  : std_logic_vector(11 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 400 ns else clk;
  process (clk)
    variable x : r8;
    variable y : r16;
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      x.d := std_logic_vector(to_unsigned(k, 8));
      y.d := std_logic_vector(to_unsigned(k * 257, 16));
      x.n := k;
      y.n := x.n * 2;
      x.e := y.d(3 downto 0);
      y.e := x.d(7 downto 4) & x.e;
      q8 <= x.d xor y.d(7 downto 0);
      q16 <= y.d(15 downto 8) & x.d;
      qe <= x.e & y.e;
      report integer'image(x.n) & " " & integer'image(y.n) & " " & to_hstring(q8) & " " & to_hstring(q16) & " " & to_hstring(qe) & " " & to_hstring(x.d) & to_hstring(y.e);
    end if;
  end process;
end;
"#;

#[test]
fn record_layouts_kept_in_the_frame_match_the_interpreter() {
    let dir = tempdir("compiled_records");
    std::fs::write(dir.join("lc.vhd"), RECORDS).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "lc.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &str| {
        // `auto` compiles at once (and keeps layouts in the frame as `llvm` does, #532).
        let tier = format!("--tier={tier}");
        let mut args = vec!["run", "--work=work:work", &tier];
        if tier == "--tier=auto" {
            args.push("--tier-heat=1,0,0");
        }
        args.push("lc");
        let o = yavs(&dir, &args);
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        )
    };
    let want = go("interp");
    assert!(want.1.matches("** Note").count() >= 20, "{want:?}");
    for tier in ["llvm", "auto"] {
        assert_eq!(go(tier), want, "{tier}");
    }
}
