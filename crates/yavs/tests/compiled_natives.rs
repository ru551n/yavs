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
