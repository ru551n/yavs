//! Speculative two-state code (`YAVS_SPEC01`, docs/design/two-state.md): compiled processes that
//! read logic vectors on the assumption of `'0'`/`'1'` give the interpreter's output with every
//! one of the nine values in every position, in shadow mode too (a mismatch of the two-state
//! result with the general one ends the run with status 70), and a process that keeps reading
//! a metavalue sleeps (back-off) without changing the output.
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee;
use ieee.std_logic_1164.all;
entity t is end;
architecture a of t is
  signal clk : std_logic := '0';
  signal v1  : std_logic_vector(0 downto 0) := "0";
  signal v7  : std_logic_vector(6 downto 0) := (others => '0');
  signal v9  : std_logic_vector(8 downto 0) := (others => '1');
  signal v33 : std_logic_vector(32 downto 0) := (others => '0');
  signal v64 : std_logic_vector(63 downto 0) := (others => '1');
  signal stuck : std_logic_vector(11 downto 0) := (others => 'X');
  signal sum : integer := 0;
begin
  clk <= not clk after 5 ns when now < 3 us;
  -- every value of the nine, moving through every position, at times all 0/1
  stim : process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      v1 <= (others => std_logic'val(2 + n mod 2));
      v7 <= (others => '0');
      v9 <= (others => '1');
      v33 <= (others => '1');
      v64 <= (others => '0');
      if n mod 5 /= 0 then
        v7(n mod 7) <= std_logic'val(2 + n mod 2);
      else
        v7(n mod 7) <= std_logic'val(n mod 9);
      end if;
      if n mod 11 = 0 then
        v9(n mod 9) <= std_logic'val((n / 11) mod 9);
      end if;
      if n mod 13 = 0 then
        v33(n mod 33) <= std_logic'val((n / 13) mod 9);
      end if;
      if n mod 17 = 0 then
        v64(n mod 64) <= std_logic'val((n / 17) mod 9);
      end if;
    end if;
  end process;
  -- reads whole signals and slices at odd offsets and lengths
  rd : process (clk)
    variable x : std_logic_vector(6 downto 0);
    variable y : std_logic_vector(5 downto 0);
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      x := v7 xor not v7(0) & v7(6 downto 1);
      y := v9(7 downto 2) and v33(5 downto 0);
      report to_string(v1) & " " & to_string(v7) & " " & to_string(v9) & " " & to_string(x) & " " & to_string(y) & " " & to_string(v33(32 downto 25)) & " " & to_string(v64(63 downto 3)) & " " & to_string(v64);
    end if;
  end process;
  -- a read of a metavalue every cycle
  st : process (clk)
    variable z : std_logic_vector(11 downto 0);
  begin
    if rising_edge(clk) then
      z := stuck xor (stuck'range => '1');
      if z(0) = 'X' then sum <= sum + 1; end if;
    end if;
  end process;
  process begin
    wait for 2999 ns;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

fn run(dir: &Path, tier: &str, mode: &str) -> (Option<i32>, String, String) {
    let o = yavs(
        dir,
        &[
            "run",
            "--work=work:work",
            &format!("--tier={tier}"),
            "--time-report",
            "t",
        ],
        &[("YAVS_SPEC01", mode), ("YAVS_SPEC01_STATS", "1")],
    );
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn stat(err: &str, key: &str) -> u64 {
    let line = err
        .lines()
        .find(|l| l.starts_with("spec01: mode="))
        .unwrap_or_else(|| panic!("no spec01 line in {err}"));
    let at = line
        .find(key)
        .unwrap_or_else(|| panic!("no {key} in {line}"))
        + key.len();
    line[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|d| d.parse().ok())
        .unwrap_or(0)
}

#[test]
fn two_state_reads_match_the_interpreter() {
    let dir = tempdir("spec01");
    std::fs::write(dir.join("t.vhd"), SRC).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "t.vhd"], &[]);
    assert!(a.status.success(), "{a:?}");
    let want = run(&dir, "interp", "off");
    assert_eq!(want.0, Some(0), "{want:?}");
    assert!(want.1.contains("sum="), "{want:?}");
    // Every nine-value position shows up in the output of the reads.
    for v in ['U', 'X', 'Z', 'W', 'L', 'H', '-'] {
        assert!(want.1.contains(v), "no {v} in the reads: {want:?}");
    }
    for mode in ["off", "on", "shadow"] {
        let got = run(&dir, "llvm", mode);
        assert_eq!((got.0, &got.1), (want.0, &want.1), "llvm, mode {mode}");
        if mode != "off" {
            // The guard failed (the stimulus has non-01 values) and the process went on in the
            // interpreter, and the permanent metavalue made `st` sleep.
            assert!(stat(&got.2, "bails_guard=") > 0, "{mode}: {}", got.2);
            assert!(stat(&got.2, "skipped_by_backoff=") > 0, "{mode}: {}", got.2);
            assert!(stat(&got.2, "activations=") > 0, "{mode}: {}", got.2);
        }
    }
    // The structural cases leave instead of joining: same output.
    let o = yavs(
        &dir,
        &["run", "--work=work:work", "--tier=llvm", "t"],
        &[("YAVS_SPEC01", "shadow"), ("YAVS_SPEC01_STRUCT", "exit")],
    );
    assert_eq!(String::from_utf8_lossy(&o.stdout), want.1);
}
