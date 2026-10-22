//! Natives, concatenations and layouts in compiled code (`--tier=llvm`, a build with the `llvm`
//! feature) against the interpreter: output and exit status are the same, also when a native
//! ends the simulation (a warning of `numeric_std` at the stop severity).
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
