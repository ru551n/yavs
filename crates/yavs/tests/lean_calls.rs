//! Calls that pass their parameters in registers (`Pre::lean`, #730): `out` and `inout` scalars,
//! nested functions and procedures reading and writing the variables of the enclosing process,
//! recursion through the static link, a callee that allocates and one that deallocates. The
//! values are those of the calls with their slots written (the reference simulator agrees).

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = r#"-- Calls whose parameters go to registers only: by-reference scalars (out, inout), nested
-- functions and procedures reading and writing the enclosing process's variables, a nested
-- function that calls a function of its parent's level, recursion through the static link, and
-- a callee that allocates.
entity lean_calls is end;
architecture a of lean_calls is
  type int_ptr is access integer;
  procedure step (variable v : inout integer; variable o : out integer; k : in integer) is
  begin
    o := v * 2 + k;
    v := (v + o) mod 1000;
    o := o + 1;
  end procedure;
  procedure grow (variable v : inout integer; variable p : inout int_ptr; k : in integer) is
  begin
    deallocate(p);
    p := new integer'(v + k);
    v := p.all + 1;
  end procedure;
begin
  process
    variable base : integer := 7;
    variable acc : integer := 0;
    variable v, o : integer := 1;
    variable p : int_ptr := new integer'(5);
    impure function add_base (k : integer) return integer is
    begin
      return base + k;
    end function;
    impure function twice (k : integer) return integer is
      impure function inner (j : integer) return integer is
      begin
        return add_base(j) + base;
      end function;
    begin
      return inner(k) * 2;
    end function;
    procedure bump (variable x : inout integer; k : integer) is
    begin
      base := base + 1;
      x := x + base + k;
    end procedure;
    function fact (n : integer) return integer is
    begin
      if n <= 1 then return 1; else return n * fact(n - 1) mod 10007; end if;
    end function;
  begin
    for i in 1 to 20 loop
      step(v, o, i);
      acc := (acc + o + v) mod 100003;
      acc := (acc + add_base(i) + twice(i)) mod 100003;
      bump(acc, i);
      grow(v, p, i);
      acc := (acc + p.all + fact(i)) mod 100003;
      report integer'image(i) & " " & integer'image(v) & " " & integer'image(o) & " " & integer'image(acc) & " " & integer'image(base);
    end loop;
    wait;
  end process;
end;
"#;

const WANT: &str = "1 6 4 61 8
2 23 15 177 9
3 76 50 447 10
4 237 157 1173 11
5 722 480 3297 12
6 179 1451 5915 13
7 552 366 12523 14
8 673 1113 15381 15
9 38 1356 19557 16
10 135 87 26305 17
11 428 282 36461 18
12 309 869 44629 19
13 954 632 52259 20
14 891 1923 65215 21
15 704 1798 74850 22
16 145 1425 76818 23
17 470 308 79512 24
18 447 959 84192 25
19 380 914 96014 26
20 181 781 6076 27";

#[test]
fn lean_calls_of_references_and_nested_subprograms_keep_their_values() {
    let dir = tempdir("lean_calls");
    std::fs::write(dir.join("lean_calls.vhd"), SRC).expect("write");
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run yavs")
    };
    let a = yavs(&[
        "analyze",
        "--std=2008",
        "--work=work:work",
        "lean_calls.vhd",
    ]);
    assert!(a.status.success(), "{a:?}");
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["--tier=interp", "--tier=llvm", "--tier=auto"]
    } else {
        &["--tier=interp"]
    };
    for tier in tiers {
        let o = yavs(&["run", "--std=2008", "--work=work:work", tier, "lean_calls"]);
        assert!(o.status.success(), "{tier}: {o:?}");
        // "** Note: <time>: <text>" lines; the text is what the process reported.
        let got: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("** Note: "))
            .map(|l| l.split_once(": ").map_or(l, |x| x.1).to_string())
            .collect();
        let want: Vec<&str> = WANT.lines().collect();
        assert_eq!(got, want, "{tier}");
    }
}
