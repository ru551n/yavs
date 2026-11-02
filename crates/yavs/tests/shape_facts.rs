//! Shape folded from what an instance fixes: temporaries of a process with nested subprograms in
//! its frame, a length check that folds true or fails with its own text, and stored copies that
//! never serve an instance with other generics.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
entity sub is
  generic (w : natural := 9; bad : boolean := false);
  port (clk : in bit; q : out bit_vector(w - 1 downto 0));
end entity;

architecture a of sub is
  signal chk : bit_vector(w - 1 downto 0);
begin
  q <= chk;
  process (clk)
    variable col : bit_vector(w - 1 downto 0) := (others => '0');
    variable n : natural := 0;
    variable sum : natural := 0;
    procedure bump (v : in bit_vector) is
    begin
      -- Reads and writes the frame of the process while its temporaries are alive.
      n := n + 1;
      for i in v'range loop
        if v(i) = '1' then
          sum := (sum * 3 + i + 1) mod 1000003;
        end if;
      end loop;
    end procedure;
    function width_of (v : bit_vector) return natural is
    begin
      return v'length;
    end function;
    function make return bit_vector is
      variable r : bit_vector(w downto 0) := (others => '1');
    begin
      return r;
    end function;
  begin
    if clk = '1' then
      bump(col & '1');
      bump(not col);
      col(n mod w) := not col(n mod w);
      chk <= col;
      if bad and n = 8 then
        col := make;
      end if;
      if n mod 16 = 0 then
        report integer'image(w) & " " & integer'image(sum) & " " & integer'image(width_of(col));
      end if;
    end if;
  end process;
end architecture;

entity top is
  generic (wa : natural := 9; wb : natural := 5; bad : boolean := false);
end entity;

architecture a of top is
  signal clk : bit := '0';
  signal qa : bit_vector(wa - 1 downto 0);
  signal qb : bit_vector(wb - 1 downto 0);
begin
  ua : entity work.sub generic map (wa, bad) port map (clk, qa);
  ub : entity work.sub generic map (wb, false) port map (clk, qb);
  clk <= not clk after 5 ns;
  process
  begin
    wait for 1 us;
    report "end " & integer'image(wa) & " " & integer'image(wb);
    std.env.finish;
  end process;
end architecture;
"#;

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const SPEC: (&str, &str) = ("YAVS_SPEC_AFTER", "2");
const EAGER: [&str; 2] = ["--tier=auto", "--tier-heat=1,0,0"];

fn prepared(name: &str) -> (common::Tmp, String) {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    (d, work)
}

fn run(
    d: &Path,
    work: &str,
    env: &[(&str, &str)],
    tier: &[&str],
    extra: &[&str],
    gens: &[&str],
) -> (i32, String, String) {
    let mut args = vec!["run", "--std=2008", "--work", work];
    args.extend(tier);
    args.extend(extra);
    args.extend(["--threads", "1", "top"]);
    args.extend(gens);
    yavs(d, env, &args)
}

/// A stored copy is made for its instance data: a warm run with other generics gets the results
/// of a cold run with those generics, not those of the instance the store was filled for.
#[test]
fn a_warm_run_with_other_generics_is_a_cold_run_with_them() {
    let (d, work) = prepared("shape_facts_warm");
    let other = ["-gwa=11", "-gwb=4"];
    let expected = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &other);
    assert_eq!(expected.0, 0, "{}", expected.2);
    assert!(expected.2.contains("end 11 4") || expected.1.contains("end 11 4"));
    let first = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!(first.0, 0, "{}", first.2);
    let warm = run(&d, &work, &[SPEC], &EAGER, &[], &other);
    assert_eq!(
        (&expected.0, &expected.1, &expected.2),
        (&warm.0, &warm.1, &warm.2)
    );
    // And back: the first entry still serves its own generics.
    let base = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &[]);
    let again = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!((&base.0, &base.1, &base.2), (&again.0, &again.1, &again.2));
}

/// Every tier gives what the interpreter gives, and a length that does not match fails at its
/// own assignment with its own text.
#[test]
fn folded_shapes_change_no_result_and_no_error_text() {
    let (d, work) = prepared("shape_facts_tiers");
    let interp = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &[]);
    assert_eq!(interp.0, 0, "{}", interp.2);
    let eager = run(&d, &work, &[SPEC], &EAGER, &["--no-cache"], &[]);
    assert_eq!(
        (&interp.0, &interp.1, &interp.2),
        (&eager.0, &eager.1, &eager.2)
    );
    let g = ["-gbad=true"];
    let interp = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &g);
    assert_ne!(interp.0, 0, "the length must fail: {}", interp.2);
    assert!(interp.2.contains("length"), "{}", interp.2);
    let eager = run(&d, &work, &[SPEC], &EAGER, &["--no-cache"], &g);
    assert_eq!(
        (&interp.0, &interp.1, &interp.2),
        (&eager.0, &eager.1, &eager.2)
    );
}

const FORMALS: &str = r#"
package p is
  type rc is record
    a : bit_vector(3 downto 0);
    b : bit_vector(0 to 4);
  end record;
  type rv is array (2 downto 0) of rc;
  subtype w8 is bit_vector(7 downto 0);
  function first(x : bit_vector(7 downto 0)) return integer;
  function sum(x : rv) return integer;
  procedure flip(x : inout w8);
  procedure bad_index(x : bit_vector(7 downto 0); i : integer);
end package;

package body p is
  function first(x : bit_vector(7 downto 0)) return integer is
  begin
    assert x'left = 7 and x'right = 0 and not x'ascending report "bounds of the formal" severity failure;
    if x(7) = '1' then return 1; else return 0; end if;
  end function;
  function sum(x : rv) return integer is
    variable n : integer := 0;
  begin
    assert x'left = 2 and x'length = 3 report "bounds of the record array" severity failure;
    for i in x'range loop
      if x(i).a(3) = '1' then n := n + 1; end if;
      if x(i).b(4) = '1' then n := n + 10; end if;
    end loop;
    return n;
  end function;
  procedure flip(x : inout w8) is
  begin
    assert x'left = 7 report "bounds of the inout formal" severity failure;
    x(7) := not x(7);
    x(0) := not x(0);
  end procedure;
  procedure bad_index(x : bit_vector(7 downto 0); i : integer) is
  begin
    assert x(i) = '0' or x(i) = '1';
  end procedure;
end package body;

use work.p.all;
entity top is
  generic (index : integer := 3; wide : boolean := false);
end entity;

architecture a of top is
  signal v : bit_vector(0 to 7) := "10000001";
  signal r : rv := (others => (a => "1000", b => "00001"));
begin
  process
    variable lv : bit_vector(0 to 7) := "10000000";
    variable lw : bit_vector(0 to 8) := "100000000";
    variable k : integer := 0;
  begin
    report integer'image(first(lv)) & " " & integer'image(first(v)) & " " & integer'image(sum(r));
    flip(lv);
    report integer'image(first(lv)) & " " & integer'image(first(lv(0 to 7)));
    bad_index(lv, index);
    if wide then
      k := first(lw);
    end if;
    report "end " & integer'image(k);
    wait;
  end process;
end architecture;
"#;

/// A formal of a static subtype reads its bounds from the pool: an actual of other bounds shows
/// the formal's, a slice and an `inout` work, a failing index and a failing length keep their text.
#[test]
fn a_formal_of_a_static_subtype_has_its_own_bounds_in_every_tier() {
    let d = tempdir("shape_facts_formals");
    std::fs::write(d.join("f.vhd"), FORMALS).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "f.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    for (gens, ok, text) in [
        (&[][..], true, "end 0"),
        (&["-gindex=9"][..], false, "index"),
        (&["-gwide=true"][..], false, "length"),
    ] {
        let interp = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], gens);
        let all = format!("{}{}", interp.1, interp.2);
        assert_eq!(interp.0 == 0, ok, "{all}");
        assert!(all.contains(text), "{all}");
        for (env, tier) in [(&[SPEC][..], &EAGER[..]), (&[][..], &["--tier=clift"][..])] {
            let got = run(&d, &work, env, tier, &["--no-cache"], gens);
            assert_eq!(
                (&interp.0, &interp.1, &interp.2),
                (&got.0, &got.1, &got.2),
                "{tier:?} {gens:?}"
            );
        }
    }
}
