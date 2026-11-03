//! `math_real.uniform` computed in place in Cranelift code (`Gen::uniform_fast`) against the
//! interpreter: the seeds and the real in process variables, passed by reference to a nested
//! procedure (frame mode) and to an architecture procedure (native frame), with the two
//! out-of-range seeds the native reports.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const UNIFORM: &str = r#"library ieee;
use ieee.math_real.all;
entity uni is end;
architecture a of uni is
  procedure draw(variable a, b : inout positive; variable x : inout real) is
  begin
    uniform(a, b, x);
  end procedure;
begin
  process
    variable s1 : positive := 123456;
    variable s2 : positive := 987654;
    variable x  : real := 0.0;
    variable acc : real := 0.0;
    procedure local is
    begin
      uniform(s1, s2, x);
      acc := acc + x;
    end procedure;
  begin
    for i in 1 to 500 loop
      local;
      draw(s1, s2, x);
      acc := acc + x;
    end loop;
    report "acc=" & real'image(acc) & " s1=" & integer'image(s1) & " s2=" & integer'image(s2);
    -- Both ends of the seeds' range, where a lane wraps and where the difference does.
    s1 := 1;
    s2 := 2147483398;
    local;
    report "low high x=" & real'image(x) & " s1=" & integer'image(s1) & " s2=" & integer'image(s2);
    s1 := 2147483562;
    s2 := 1;
    draw(s1, s2, x);
    report "high low x=" & real'image(x) & " s1=" & integer'image(s1) & " s2=" & integer'image(s2);
    s1 := positive'high;
    local;
    report "bad seed1 x=" & real'image(x) & " s1=" & integer'image(s1);
    s1 := 5;
    s2 := positive'high;
    draw(s1, s2, x);
    report "bad seed2 x=" & real'image(x) & " s2=" & integer'image(s2);
    wait;
  end process;
end;
"#;

fn go(dir: &Path, flags: &[&str]) -> (Option<i32>, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["run", "--work=work:work"])
        .args(flags)
        .arg("uni")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn uniform_in_cranelift_code_matches_the_interpreter() {
    let dir = tempdir("clift_uniform");
    std::fs::write(dir.join("uni.vhd"), UNIFORM).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--std=2008", "--work=work:work", "uni.vhd"])
        .current_dir(&dir)
        .output()
        .expect("run yavs");
    assert!(a.status.success(), "{a:?}");
    let want = go(&dir, &["--tier=interp"]);
    for line in [
        "acc=",
        "low high x=",
        "high low x=",
        "SEED1 > 2147483562 in UNIFORM",
        "SEED2 > 2147483398 in UNIFORM",
    ] {
        assert!(want.1.contains(line), "{want:?}");
    }
    assert_eq!(go(&dir, &["--tier=clift"]), want, "clift");
    // `auto` compiles at once.
    assert_eq!(
        go(&dir, &["--tier=auto", "--tier-heat=1,0,0"]),
        want,
        "auto"
    );
}
