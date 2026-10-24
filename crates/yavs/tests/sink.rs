//! Sinking of a check's message into the branch that reads it (`opt/sink.rs`): a check whose
//! message is built from images, lengths and a concatenation builds it only for a failing call,
//! and everything the message does that anyone can see (a failure, a report, a call) still
//! happens at every call. Each design runs with the optimiser and without it and must print
//! the same, failures included. The second half is the `to_integer` of a check's condition
//! and of its message (`opt/sink/calls.rs`): a second call on the same value is made again
//! only when the first returned 0, and the warnings of a vector with `U` or of a null vector
//! come twice, in the same place among the other output.

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
        "the same to_integer in the condition and the message",
        "for k in 40 to 44 loop check(to_integer(unsigned(v)) = k, \"q\" & integer'image(to_integer(unsigned(v))) & integer'image(k)); end loop;",
    ),
    (
        "a vector with U: both calls warn",
        "for k in 0 to 3 loop check(to_integer(unsigned(w)) = k, \"m\" & integer'image(to_integer(unsigned(w))) & integer'image(k)); end loop;",
    ),
    (
        "a vector of zeros: both calls run",
        "for k in 0 to 2 loop check(to_integer(unsigned(z)) = k, \"z\" & integer'image(to_integer(unsigned(z))) & integer'image(k)); end loop;",
    ),
    (
        "a null vector warns twice",
        "for k in 0 to 2 loop check(to_integer(unsigned(w(0 downto 1))) = k, \"n\" & integer'image(to_integer(unsigned(w(0 downto 1)))) & integer'image(k)); end loop;",
    ),
    (
        "another warning between the two calls",
        "for k in 0 to 2 loop check(to_integer(unsigned(w)) = k, \"b\" & integer'image(to_integer(unsigned(u))) & integer'image(to_integer(unsigned(w)))); end loop;",
    ),
    (
        "another warning between two calls of a valid vector",
        "for k in 40 to 43 loop check(to_integer(unsigned(v)) = k, \"b\" & integer'image(to_integer(unsigned(u))) & integer'image(to_integer(unsigned(v)))); end loop;",
    ),
    (
        "three calls on one value",
        "for k in 0 to 2 loop check(to_integer(unsigned(w)) = k, \"t\" & integer'image(to_integer(unsigned(w))) & integer'image(to_integer(unsigned(w)) + k)); end loop;",
    ),
    (
        "a signed vector is left alone",
        "for k in -1 to 1 loop check(to_integer(signed(one)) = k, \"s\" & integer'image(to_integer(signed(one))) & integer'image(to_integer(signed(w)))); end loop;",
    ),
    (
        "the signal changes between statements",
        "for k in 0 to 1 loop check(to_integer(unsigned(w)) = k, \"p\" & integer'image(to_integer(unsigned(w)))); end loop; w <= \"0110\"; wait for 1 ns; for k in 5 to 7 loop check(to_integer(unsigned(w)) = k, \"r\" & integer'image(to_integer(unsigned(w)))); end loop; w <= \"0U10\"; wait for 1 ns; check(to_integer(unsigned(w)) = 6, \"s\" & integer'image(to_integer(unsigned(w))));",
    ),
    (
        "nested loops, some checks failing",
        "for k in 0 to 3 loop for j in 0 to 2 loop check(k /= j, \"k\" & integer'image(k) & \"j\" & integer'image(j) & color'image(color'val(j)) & integer'image(to_integer(unsigned(v)) + k)); end loop; end loop;",
    ),
];

fn run(dir: &std::path::Path, env: &[(&str, &str)], top: &str, tier: &str) -> (i32, String) {
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
        .args([&format!("--tier={tier}"), top])
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

fn same_output_with_and_without_the_optimiser(tier: &str) {
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
  signal u : std_logic_vector(1 downto 0) := \"1X\";
  signal z : std_logic_vector(3 downto 0) := \"0000\";
  signal one : std_logic_vector(0 downto 0) := \"1\";
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
        let with = run(&d, &[], "t", tier);
        let without = run(&d, &[("YAVS_NO_IROPT", "1")], "t", "interp");
        assert_eq!(with, without, "{what} ({tier})");
    }
}

#[test]
fn a_message_built_for_a_check_changes_nothing_anyone_sees() {
    same_output_with_and_without_the_optimiser("interp");
}

#[cfg(feature = "llvm")]
#[test]
fn the_same_in_compiled_code() {
    same_output_with_and_without_the_optimiser("llvm");
}

#[test]
fn the_second_to_integer_is_a_replay_behind_a_guard() {
    let d = tmp("sink");
    let text = format!(
        "{PACKAGE}
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.chk.all;
entity t is end;
architecture a of t is
  signal v : std_logic_vector(7 downto 0) := x\"2a\";
begin
  process
  begin
    for k in 40 to 43 loop check(to_integer(unsigned(v)) = k, \"q\" & integer'image(to_integer(unsigned(v)))); end loop;
    wait;
  end process;
end;
"
    );
    std::fs::write(d.join("t.vhd"), text).expect("write");
    // The replay: the second call is alone in a block that jumps on.
    let replays = |env: &[(&str, &str)]| {
        let (_, dump) = run(&d, env, "t", "interp");
        let lines: Vec<&str> = dump.lines().collect();
        lines
            .windows(2)
            .filter(|w| w[0].contains("CallNative { native: n0") && w[1].contains(" Br {"))
            .count()
    };
    assert_eq!(replays(&[("YAVS_DUMPIR", "1")]), 1);
    assert_eq!(replays(&[("YAVS_DUMPIR", "1"), ("YAVS_NO_IROPT", "1")]), 0);
}

/// A call inside inlined code (the method of a protected type, called by a wrapper the process
/// inlines) is inlined in its turn, with its message built in the caller: the instructions the
/// inlining adds at the call keep the call's statement and the calls it came through. (The IR
/// verifier of a debug build rejects a statement of another function.) The method is larger
/// than a plain callee allows before the optimiser folds `verbose` away.
#[test]
fn a_call_inside_inlined_code_is_inlined_with_the_calls_it_came_through() {
    let d = tmp("sink");
    let text = "
package chk is
  type status_t is protected
    procedure assertion(condition : boolean; message : string := \"\");
    impure function failed return natural;
  end protected;
  procedure check(condition : boolean; message : string := \"\");
  impure function failures return natural;
end package;

package body chk is
  constant verbose : boolean := false;
  type status_t is protected body
    variable total, failed_n, streak : natural := 0;
    procedure fail(message : string := \"\") is
    begin
      if message'length > 0 then report message severity warning; end if;
    end procedure;
    procedure assertion(condition : boolean; message : string := \"\") is
    begin
      total := total + 1;
      if verbose then
        report \"total=\" & integer'image(total) & \" failed=\" & integer'image(failed_n) & \" streak=\" & integer'image(streak) severity note;
      end if;
      if not condition then
        fail(message);
        failed_n := failed_n + 1;
        if failed_n >= 100 then streak := 0; end if;
      end if;
    end procedure;
    impure function failed return natural is begin return failed_n; end function;
  end protected body;
  shared variable st : status_t;
  procedure check(condition : boolean; message : string := \"\") is
  begin
    st.assertion(condition, message);
  end procedure;
  impure function failures return natural is begin return st.failed; end function;
end package body;

use work.chk.all;
entity t is end;
architecture a of t is
begin
  process
  begin
    for k in 1 to 6 loop
      check(k mod 3 /= 0, \"k=\" & integer'image(k) & \" of \" & integer'image(k * 7));
    end loop;
    report \"failed=\" & integer'image(failures);
    wait;
  end process;
end;
";
    std::fs::write(d.join("t.vhd"), text).expect("write");
    let with = run(&d, &[], "t", "interp");
    let without = run(&d, &[("YAVS_NO_IROPT", "1")], "t", "interp");
    assert_eq!(with.0, 0, "{}", with.1);
    assert_eq!(with, without);
}

/// A constant two branches share (one register, defined once, in the first branch) keeps its
/// definition when inlining folds that branch away (#714): the index read 0 in a release build
/// and the IR verifier of a debug build rejected the function.
#[test]
fn a_constant_shared_by_two_branches_survives_the_folding_of_one() {
    let d = tmp("sink");
    let text = "
package pk is
  procedure put(i : natural; v : integer);
  procedure setg(flag : boolean);
  procedure gate;
end package;
package body pk is
  procedure put(i : natural; v : integer) is
  begin
    report \"put \" & integer'image(i) & \"=\" & integer'image(v) severity note;
  end procedure;
  procedure setg(flag : boolean) is
  begin
    if flag then put(20, 1); else put(20, 0); end if;
  end procedure;
  procedure gate is
  begin
    report \"gate a\" & integer'image(1) severity note;
    report \"gate b\" & integer'image(2) severity note;
    report \"gate c\" & integer'image(3) severity note;
    report \"gate d\" & integer'image(4) severity note;
    setg(false);
    report \"gate e\" & integer'image(5) severity note;
  end procedure;
end package body;
use work.pk.all;
entity t is end;
architecture a of t is
begin
  process
  begin
    gate;
    wait;
  end process;
end;
";
    std::fs::write(d.join("t.vhd"), text).expect("write");
    let with = run(&d, &[], "t", "interp");
    let without = run(&d, &[("YAVS_NO_IROPT", "1")], "t", "interp");
    assert_eq!(with.0, 0, "{}", with.1);
    assert!(with.1.contains("put 20=0"), "{}", with.1);
    assert_eq!(with, without);
}
