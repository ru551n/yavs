//! Copies of composite values: between two objects of one heap (access values), a logic vector
//! range between two designated vectors, and whole records through designated objects (#677).

mod common;
use common::tempdir as tmp;

use std::process::Command;

const COPIES: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity copies is end;
architecture a of copies is
  type slv_ptr is access std_logic_vector;
  type rec is record
    a : integer;
    v : std_logic_vector(0 to 299);
    s : string(1 to 5);
  end record;
  type rec_ptr is access rec;
begin
  process
    variable p, q : slv_ptr;
    variable r, t : rec_ptr;
    variable ok : boolean := true;
  begin
    p := new std_logic_vector(0 to 999);
    q := new std_logic_vector(0 to 999);
    for i in p'range loop
      if i mod 3 = 0 then p(i) := '1'; elsif i mod 3 = 1 then p(i) := 'Z'; else p(i) := 'X'; end if;
      q(i) := '0';
    end loop;
    -- aligned, unaligned and short ranges from one designated vector to another
    q(64 to 575) := p(64 to 575);
    q(3 to 12) := p(500 to 509);
    q(700 to 999) := p(701 to 1000 - 1) & '1';
    for i in q'range loop
      if i >= 64 and i <= 575 then ok := ok and q(i) = p(i);
      elsif i >= 3 and i <= 12 then ok := ok and q(i) = p(i + 497);
      elsif i >= 700 and i <= 998 then ok := ok and q(i) = p(i + 1);
      elsif i = 999 then ok := ok and q(i) = '1';
      else ok := ok and q(i) = '0';
      end if;
    end loop;
    assert ok report \"vector ranges differ\" severity failure;
    -- whole records between two designated objects
    r := new rec;
    t := new rec;
    r.a := 42;
    r.v := (others => '1');
    r.v(100 to 199) := (others => 'H');
    r.s := \"hello\";
    t.all := r.all;
    assert t.a = 42 and t.v(150) = 'H' and t.v(250) = '1' and t.s = \"hello\" report \"record copy\" severity failure;
    r.a := 7;
    assert t.a = 42 report \"copy shares storage\" severity failure;
    t.all := t.all;
    assert t.a = 42 and t.s = \"hello\" report \"self copy\" severity failure;
    report \"copies ok\";
    wait;
  end process;
end;
";

/// A record signal whose scalar leaves lie next to each other (runs of `std_logic`, `bit` and
/// `boolean` fields, all nine `std_ulogic` values) read whole into a variable and driven whole
/// from one: every field comes back as it went in, in steps of one delta cycle.
const RUNS: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity runs is end;
architecture a of runs is
  type rec is record
    a0, a1, a2, a3, a4, a5, a6, a7, a8, a9 : std_logic;
    b0, b1, b2, b3, b4, b5 : bit;
    c : integer range 0 to 255;
    d0, d1, d2, d3 : std_logic;
    e : std_logic_vector(7 downto 0);
    f0, f1, f2, f3 : boolean;
    g0, g1, g2 : std_logic;
  end record;
  constant init : rec := (
    'U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-', '1',
    '1', '0', '1', '0', '0', '1', 200,
    '0', '1', 'Z', 'X', \"10Z1XWLH\",
    true, false, false, true, '1', 'L', '-');
  signal r : rec := init;
  signal s : rec;
begin
  process
    variable v : rec;
    variable ok : boolean := true;
  begin
    v := r;
    assert v = init report \"first gather\" severity failure;
    v.a3 := 'Z'; v.a9 := 'X'; v.b2 := '0'; v.b3 := '1'; v.f1 := true; v.f2 := true;
    v.d0 := '1'; v.g1 := '0'; v.c := 17; v.e(3) := '0';
    s <= v;
    wait for 1 ns;
    assert s.a0 = 'U' and s.a1 = 'X' and s.a2 = '0' and s.a3 = 'Z' and s.a4 = 'Z' and s.a5 = 'W'
       and s.a6 = 'L' and s.a7 = 'H' and s.a8 = '-' and s.a9 = 'X' report \"a run\" severity failure;
    assert s.b0 = '1' and s.b1 = '0' and s.b2 = '0' and s.b3 = '1' and s.b4 = '0' and s.b5 = '1'
       report \"bit run\" severity failure;
    assert s.d0 = '1' and s.d1 = '1' and s.d2 = 'Z' and s.d3 = 'X' and s.c = 17 report \"d run\" severity failure;
    assert s.f0 = true and s.f1 = true and s.f2 = true and s.f3 = true report \"boolean run\" severity failure;
    assert s.g0 = '1' and s.g1 = '0' and s.g2 = '-' and s.e = \"10Z10WLH\" report \"tail\" severity failure;
    -- a whole read of a signal driven whole, then a field driven after it in the same cycle
    v := s;
    assert v = s report \"second gather\" severity failure;
    r <= v;
    r.a5 <= '1';
    wait for 1 ns;
    assert r.a4 = 'Z' and r.a5 = '1' and r.a6 = 'L' and r.b3 = '1' and r.f1 = true report \"mixed\" severity failure;
    report \"runs ok\";
    wait;
  end process;
end;
";

/// A record signal with an array of records inside, read whole and driven whole (#790): the
/// instances of a logic leaf lie mirrored in the kernel and go as one run of the driver; byte
/// leaves (integers) between them go one by one. Elements read on their own see the same values.
const INSTANCES: &str = "
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity insts is end;
architecture a of insts is
  type ent is record
    v : std_logic;
    tag : std_logic_vector(5 downto 0);
    n : integer range 0 to 1000;
    d : std_logic_vector(0 to 2);
  end record;
  type ents is array (0 to 4) of ent;
  type rec is record
    head : std_logic;
    e : ents;
    tail : std_logic_vector(3 downto 0);
  end record;
  signal r : rec := ('1', (others => ('0', \"000000\", 0, \"000\")), \"0000\");
begin
  process
    variable v : rec;
    variable x : ent;
  begin
    for k in 1 to 3 loop
      v := r;
      for i in v.e'range loop
        v.e(i).v := not v.e(i).v;
        v.e(i).tag := std_logic_vector(to_unsigned(i * 7 + k, 6));
        v.e(i).n := i * 100 + k;
        v.e(i).d := (0 => 'Z', 1 => 'H', 2 => 'X');
        if i = 3 then v.e(i).d(1) := 'L'; end if;
      end loop;
      v.head := 'W';
      v.tail := \"1Z0L\";
      r <= v;
      wait for 1 ns;
      for i in r.e'range loop
        x := r.e(i);
        assert x.n = i * 100 + k and x.tag = std_logic_vector(to_unsigned(i * 7 + k, 6))
          report \"element \" & integer'image(i) severity failure;
        assert x.v = r.e(i).v and x.d(0) = 'Z' and x.d(2) = 'X' report \"fields\" severity failure;
        assert (i = 3 and x.d(1) = 'L') or (i /= 3 and x.d(1) = 'H') report \"d(1)\" severity failure;
      end loop;
      assert r.e(0).v = r.e(4).v and (k mod 2 = 1) = (r.e(2).v = '1') report \"v\" severity failure;
      assert r.head = 'W' and r.tail = \"1Z0L\" report \"ends\" severity failure;
    end loop;
    report \"instances ok\";
    wait;
  end process;
end;
";

/// A large record variable is built where it lives (frame object of a function, heap object of a
/// procedure that waits, node of an access type), not in a temporary that is copied (#792):
/// defaults, assignments and later changes of the variable stay separate objects.
const INPLACE: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity inplace is end;
architecture a of inplace is
  type rec is record
    n   : integer range -10 to 100;
    big : std_logic_vector(0 to 9999);
    s   : string(1 to 100);
  end record;
  type node;
  type node_p is access node;
  type node is record
    nxt : node_p;
    e   : rec;
  end record;
  function mk(i : integer) return rec is
    variable v : rec;
    variable w : std_logic_vector(0 to 9999) := (0 to 99 => '1', 100 to 199 => 'X', 200 to 9998 => 'U', 9999 => '0');
  begin
    assert w(0) = '1' and w(99) = '1' and w(100) = 'X' and w(199) = 'X' and w(200) = 'U' and w(5000) = 'U' and w(9998) = 'U' and w(9999) = '0' report \"aggregate default\" severity failure;
    if i > 0 then
      v.n := i;
      v.big(i) := '1';
      v.s(3) := 'x';
    end if;
    return v;
  end function;
  procedure slow(i : integer; r : out rec) is
    variable v : rec;
    variable p : node_p;
  begin
    wait for 1 ns;
    v.n := i;
    v.big(7) := 'H';
    p := new node'(nxt => null, e => v);
    v.n := 99;
    v.big(9) := '1';
    r := p.e;
    r.big(8) := '0';
    assert p.e.n = i and p.e.big(9) = 'U' and p.e.big(8) = 'U' report \"node shares the variable\" severity failure;
    assert v.big(8) = 'U' and v.big(7) = 'H' report \"variable changed by the copy\" severity failure;
    deallocate(p);
  end procedure;
begin
  process
    variable a, b : rec;
  begin
    for k in 1 to 3 loop
      a := mk(0);
      assert a.n = -10 and a.big(5) = 'U' and a.big(9999) = 'U' and a.s(1) = NUL report \"default\" severity failure;
      a := mk(4);
      b := mk(0);
      assert a.n = 4 and a.big(4) = '1' and a.big(5) = 'U' and a.s(3) = 'x' and a.s(2) = NUL report \"set\" severity failure;
      assert b.n = -10 and b.big(4) = 'U' and b.s(3) = NUL report \"calls share a variable\" severity failure;
      slow(k, a);
      assert a.n = k and a.big(7) = 'H' and a.big(8) = '0' and a.big(9) = 'U' and a.s(3) = NUL report \"waiting procedure\" severity failure;
    end loop;
    report \"inplace ok\";
    wait;
  end process;
end;
";

/// Analyse and run `src` (entity `top`) on the interpreter, and on the compiled tier when built in,
/// and expect `want` in the output.
fn check(name: &str, src: &str, top: &str, want: &str) {
    let d = tmp(name);
    std::fs::write(d.join(format!("{top}.vhd")), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args([
            "analyze",
            "--std=2008",
            "--work",
            &work,
            &format!("{top}.vhd"),
        ])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["interp", "auto"]
    } else {
        &["interp"]
    };
    for tier in tiers {
        let o = Command::new(yavs)
            .current_dir(&d)
            .args([
                "--color",
                "never",
                "run",
                "--std=2008",
                "--no-cache",
                "--work",
                &work,
            ])
            .arg(format!("--tier={tier}"))
            .arg(top)
            .output()
            .expect("run");
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(o.status.success(), "{tier}: {out}\n{err}");
        assert!(
            out.contains(want) || err.contains(want),
            "{tier}: {out}\n{err}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn copies_between_designated_objects() {
    check("value_move", COPIES, "copies", "copies ok");
}

#[test]
fn runs_of_scalar_record_leaves() {
    check("value_move_runs", RUNS, "runs", "runs ok");
}

#[test]
fn instances_of_record_leaves() {
    check("value_move_insts", INSTANCES, "insts", "instances ok");
}

#[test]
fn large_variables_built_in_place() {
    check("value_move_inplace", INPLACE, "inplace", "inplace ok");
}

/// Analyse `src` (entity `top`) and run it on the interpreter with the optimised IR dumped:
/// the dump (standard error) and the standard output.
fn run_with_dump(name: &str, src: &str, top: &str) -> (String, String) {
    let d = tmp(name);
    std::fs::write(d.join(format!("{top}.vhd")), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args([
            "analyze",
            "--std=2008",
            "--work",
            &work,
            &format!("{top}.vhd"),
        ])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "run",
            "--std=2008",
            "--no-cache",
            "--tier=interp",
            "--work",
            &work,
            top,
        ])
        .output()
        .expect("run");
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(o.status.success(), "{out}\n{err}");
    let _ = std::fs::remove_dir_all(&d);
    (out, err)
}

/// The dump of the function named `f` (in backquotes).
fn function_dump<'a>(err: &'a str, f: &str) -> &'a str {
    err.split("\nf")
        .find(|b| b.contains(f))
        .unwrap_or_else(|| panic!("no dump of {f}: {err}"))
        .split("\nops of")
        .next()
        .unwrap_or_default()
}

/// The optimiser builds the variable in its own object: the `NewFrame` of `mk` and the `NewFill`
/// of `slow` start from a zero byte (`fill: 1`) and nothing copies a temporary into them.
#[test]
fn large_variables_are_made_before_they_are_filled() {
    let (_, err) = run_with_dump("value_move_inplace_ir", INPLACE, "inplace");
    for f in ["`mk`", "`slow`"] {
        let body = function_dump(&err, f);
        assert!(
            body.contains("fill: 1"),
            "{f} is not built in place:\n{body}"
        );
        // (`mk` has a second, aggregate-initialised variable that is copied.)
        assert!(
            f == "`mk`" || !body.contains("fill: 0"),
            "{f} copies its variable from a temporary:\n{body}"
        );
    }
}

/// Arrays of `std_ulogic` separated by integer fields: the 'U' fills of a fresh zero object write
/// zeros over zeros and are not made, in a function and in a procedure that waits.
const ZEROS: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity zeros is end;
architecture a of zeros is
  type rec is record
    a1 : std_logic_vector(0 to 9999);
    n  : integer range -10 to 100;
    a2 : std_logic_vector(0 to 9999);
    m  : natural range 1 to 100;
    a3 : std_logic_vector(0 to 9999);
  end record;
  procedure proc(i : natural; r : out integer) is
    variable v : rec;
  begin
    wait for 1 ns;
    v.n := i;
    r := v.n + v.m;
  end procedure;
  function fn(i : natural) return integer is
    variable v : rec;
  begin
    v.n := i;
    return v.n + v.m;
  end function;
begin
  process
    variable x, s : integer := 0;
  begin
    for i in 1 to 5 loop
      proc(i, x);
      s := s + x + fn(i);
    end loop;
    report \"sum \" & integer'image(s);
    wait;
  end process;
end;
";

#[test]
fn fills_of_zero_over_a_fresh_object_are_dropped() {
    let (out, err) = run_with_dump("value_move_zeros", ZEROS, "zeros");
    assert!(
        out.contains("sum 40") || err.contains("sum 40"),
        "{out}\n{err}"
    );
    for f in ["`proc`", "`fn`"] {
        let body = function_dump(&err, f);
        assert!(
            body.contains("fill: 1") && !body.contains("LFill"),
            "{f} fills its fresh variable with zeros:\n{body}"
        );
    }
}
