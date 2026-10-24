//! Sinking of a check's message into the branch that reads it (`opt/sink.rs`): a check whose
//! message is built from images, lengths and a concatenation builds it only for a failing call,
//! and everything the message does that anyone can see (a failure, a report, a call) still
//! happens at every call. Each design runs with the optimiser and without it and must print
//! the same, failures included.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const PACKAGE: &str = "
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package chk is
  type status_t is protected
    procedure assertion(condition : boolean; message : string := \"\");
    impure function failed return natural;
  end protected;
  procedure check(cond : boolean; message : string := \"\");
  impure function failures return natural;
  impure function bump return integer;
  impure function bumps return integer;
end package;

package body chk is
  type status_t is protected body
    variable total  : natural := 0;
    variable failed_n : natural := 0;
    procedure fail(message : string := \"\") is
    begin
      if message'length > 0 then
        report message severity warning;
      end if;
    end procedure;
    procedure assertion(condition : boolean; message : string := \"\") is
    begin
      total := total + 1;
      if not condition then
        fail(message);
        failed_n := failed_n + 1;
      end if;
    end procedure;
    impure function failed return natural is
    begin
      return failed_n;
    end function;
  end protected body;
  type ctr_t is protected
    impure function nxt return integer;
    impure function cur return integer;
  end protected;
  type ctr_t is protected body
    variable c : integer := 0;
    impure function nxt return integer is begin c := c + 1; return c; end function;
    impure function cur return integer is begin return c; end function;
  end protected body;
  shared variable ctr : ctr_t;
  impure function bump return integer is begin return ctr.nxt; end function;
  impure function bumps return integer is begin return ctr.cur; end function;
  shared variable st : status_t;
  procedure check(cond : boolean; message : string := \"\") is
  begin
    st.assertion(cond, message);
  end procedure;
  impure function failures return natural is
  begin
    return st.failed;
  end function;
end package body;


";

/// (what the message does, the statements of the process)
const CASES: &[(&str, &str)] = &[
    (
        "division by zero in a message that is never read",
        "for k in 1 to 3 loop check(true, \"q\" & integer'image(10 / zero)); end loop;",
    ),
    (
        "overflow in a message",
        "for k in 1 to 3 loop check(true, \"q\" & integer'image(big + big)); end loop;",
    ),
    (
        "a message with a side effect",
        "for k in 1 to 4 loop check(k mod 2 = 0, \"q\" & integer'image(bump) & color'image(red) & integer'image(k)); end loop;",
    ),
    (
        "an index out of range in a message",
        "for k in 1 to 3 loop check(k < 3, \"q\" & integer'image(a(idx - 7 * k)) & \"x\"); end loop;",
    ),
    (
        "a slice out of range in a message",
        "for k in 1 to 3 loop check(true, \"q\" & s(1 to 2 + 2 * k) & integer'image(k)); end loop;",
    ),
    (
        "a to_integer that warns",
        "for k in 0 to 3 loop check(true, \"m\" & integer'image(to_integer(unsigned(w))) & integer'image(k)); end loop;",
    ),
    (
        "variables changed between calls",
        "for k in 0 to 5 loop i := k; check(i /= 3, \"i=\" & integer'image(i) & integer'image(acc)); i := i + 1; acc := acc + i; end loop;",
    ),
    (
        "nested loops, some checks failing",
        "for k in 0 to 3 loop for j in 0 to 2 loop check(k /= j, \"k\" & integer'image(k) & \"j\" & integer'image(j) & color'image(color'val(j)) & integer'image(to_integer(unsigned(v)) + k)); end loop; end loop;",
    ),
];

fn run(dir: &std::path::Path, env: &[(&str, &str)], top: &str) -> (i32, String) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", dir.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["analyze", "--std=2008", "--work", &work, "t.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(dir)
        .envs(env.iter().copied())
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", top])
        .output()
        .expect("run");
    (
        o.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

#[test]
fn a_message_built_for_a_check_changes_nothing_anyone_sees() {
    for (what, body) in CASES {
        let d = tmp("sink");
        let text = format!(
            "{PACKAGE}
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.chk.all;
entity t is end;
architecture a of t is
  type color is (red, green, blue);
  type arr_t is array (0 to 3) of integer;
  signal v : std_logic_vector(7 downto 0) := x\"2a\";
  signal w : std_logic_vector(3 downto 0) := \"01U0\";
begin
  process
    variable zero : integer := 0;
    variable big : integer := 2147483000;
    variable idx : integer := 10;
    variable a : arr_t := (1, 2, 3, 4);
    variable s : string(1 to 5) := \"hello\";
    variable acc : integer := 0;
    variable i : integer := 0;
  begin
    {body}
    report \"done acc=\" & integer'image(acc) & \" bumps=\" & integer'image(bumps) & \" failed=\" & integer'image(failures);
    wait;
  end process;
end;
"
        );
        std::fs::write(d.join("t.vhd"), text).expect("write");
        let with = run(&d, &[], "t");
        let without = run(&d, &[("YAVS_NO_IROPT", "1")], "t");
        assert_eq!(with, without, "{what}");
    }
}
