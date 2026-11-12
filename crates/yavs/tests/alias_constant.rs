//! An object alias of a constant that lives in instance data (a generic, a constant of an
//! architecture that is not locally static) or of a part of one (a record field, an element or
//! slice of a logic array): in a subprogram of the architecture, in a function called at
//! elaboration and in a process, with and without a subtype indication, in two instances with
//! different generics. A subtype that does not match the constant's length is a run-time error
//! at the alias declaration (LRM 6.6.2).

mod common;
use common::tempdir;

use std::process::{Command, Output};

const DESIGN: &str = "library ieee;
use ieee.std_logic_1164.all;

package pk is
  type rec_t is record
    a : integer;
    v : std_ulogic_vector(3 downto 0);
  end record;
  type ivec is array (natural range <>) of integer;
  function mk(n : natural) return ivec;
end package;
package body pk is
  function mk(n : natural) return ivec is
    variable r : ivec(1 to n);
  begin
    for i in r'range loop r(i) := i * 10; end loop;
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pk.all;
entity sub is
  generic (tbl : ivec; bits : std_ulogic_vector; r : rec_t; k : integer);
end entity;
architecture a of sub is
  constant c : ivec := mk(k);

  function at(i : natural) return integer is
    alias x : ivec(0 to tbl'length - 1) is tbl;
  begin
    return x(i);
  end function;

  function sum return integer is
    alias x : ivec(0 to tbl'length - 1) is tbl;
    variable s : integer := 0;
  begin
    for i in x'range loop s := s + x(i); end loop;
    return s;
  end function;
  constant total : integer := sum;

  function bits_lo return std_ulogic_vector is
    alias h : std_ulogic_vector(1 downto 0) is bits(bits'left to bits'left + 1);
  begin
    return h;
  end function;

  function rec_a return integer is
    alias ra : integer is r.a;
    alias rv : std_ulogic_vector(0 to 3) is r.v;
  begin
    return ra + 100 * boolean'pos(rv(0) = '1');
  end function;

  -- Slices of logic arrays read after other statements made temporaries: of a parameter and
  -- of a generic.
  function swap(v : std_ulogic_vector(3 downto 0)) return std_ulogic_vector is
    alias hi : std_ulogic_vector(1 downto 0) is v(3 downto 2);
    alias lo : std_ulogic_vector(1 downto 0) is v(1 downto 0);
    variable t : std_ulogic_vector(3 downto 0);
  begin
    t := not v;
    t := t xor (lo & hi);
    return lo & hi;
  end function;

  function swap_bits return std_ulogic_vector is
    alias h : std_ulogic_vector(0 to 1) is bits(bits'left to bits'left + 1);
    alias t : std_ulogic_vector(0 to 1) is bits(bits'right - 1 to bits'right);
    variable x : std_ulogic_vector(0 to 1);
  begin
    x := not h;
    x := x xor t;
    return t & h;
  end function;

  function local_c return integer is
    alias lc : ivec(c'length downto 1) is c;
  begin
    return lc(c'length) + lc'left;
  end function;
begin
  process
    alias g : ivec is tbl;
    alias b0 : std_ulogic is bits(bits'right);
    alias whole : rec_t is r;
    constant pc : ivec := mk(k + 1);
    alias pa : ivec(0 to k) is pc;
    variable i : natural := 1;
  begin
    report \"RES k=\" & integer'image(k)
      & \" at=\" & integer'image(at(i))
      & \" total=\" & integer'image(total)
      & \" g=\" & integer'image(g(g'left)) & \",\" & integer'image(g'left)
      & \" lo=\" & to_string(bits_lo)
      & \" b0=\" & std_ulogic'image(b0)
      & \" rec=\" & integer'image(rec_a) & \",\" & integer'image(whole.a)
      & \" lc=\" & integer'image(local_c)
      & \" swap=\" & to_string(swap(r.v)) & \",\" & to_string(swap_bits)
      & \" pa=\" & integer'image(pa(k)) & \",\" & integer'image(pa'length);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.pk.all;
entity top is end entity;
architecture a of top is
begin
  u1 : entity work.sub
    generic map (tbl => (3 => 7, 4 => 8, 5 => 9), bits => \"1001\", r => (a => 5, v => \"1010\"), k => 2);
  u2 : entity work.sub
    generic map (tbl => (1, 2, 3, 4), bits => \"011\", r => (a => 6, v => \"0001\"), k => 3);
end architecture;
";

const EXPECTED: &[&str] = &[
    "RES k=2 at=8 total=24 g=7,3 lo=10 b0='1' rec=105,5 lc=12 swap=1010,0110 pa=30,3",
    "RES k=3 at=2 total=10 g=1,0 lo=01 b0='1' rec=6,6 lc=13 swap=0100,1101 pa=40,4",
];

/// The alias of `crossbar`-like code in VHDL-1993: a generic of an unconstrained array type of
/// a package, seen from index 0 by a function called at elaboration.
const DESIGN_93: &str = "package pk is
  type ivec is array (natural range <>) of integer;
end package;

use work.pk.all;
entity sub is
  generic (tbl : ivec);
end entity;
architecture a of sub is
  type rvec is array (0 to 3) of integer;
  function gen return rvec is
    alias x : ivec(0 to tbl'length - 1) is tbl;
    variable ret : rvec := (others => -1);
  begin
    for i in ret'range loop
      if i < tbl'length then ret(i) := x(i); end if;
    end loop;
    return ret;
  end function;
  constant routes : rvec := gen;
begin
  process
  begin
    report \"RES \" & integer'image(routes(0)) & integer'image(routes(1))
      & integer'image(routes(2)) & integer'image(routes(3));
    wait;
  end process;
end architecture;

use work.pk.all;
entity top is end entity;
architecture a of top is
begin
  u : entity work.sub generic map (tbl => (5 => 1, 6 => 2, 7 => 3));
end architecture;
";

/// `@@` marks the alias declaration whose subtype is one element short.
const MISMATCH: &str = "entity top is
  generic (tbl : integer_vector := (1, 2, 3));
end entity;
architecture a of top is
begin
  process
    @@alias x : integer_vector(0 to 1) is tbl;
  begin
    report \"RES \" & integer'image(x(0));
    wait;
  end process;
end architecture;
";

fn tiers() -> Vec<&'static str> {
    let mut tiers = vec!["--tier=interp"];
    if cfg!(feature = "clift") {
        tiers.push("--tier=clift");
    }
    tiers
}

fn yavs(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run")
}

/// Analyse `src` as `name/case.vhd` under `tmp` and return the `--work` argument.
fn analyse(tmp: &std::path::Path, name: &str, std: &str, src: &str) -> String {
    let dir = tmp.join(name);
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("case.vhd");
    std::fs::write(&file, src).expect("write");
    let work = format!("--work=work:{}/lib", dir.display());
    let o = yavs(&["analyze", std, &work, file.to_str().expect("path")]);
    assert!(
        o.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    work
}

fn run_ok(work: &str, std: &str, name: &str, want: &[&str]) {
    for tier in tiers() {
        for threads in ["--threads=1", "--threads=4"] {
            let o = yavs(&["run", std, work, tier, threads, "top"]);
            let out = String::from_utf8_lossy(&o.stdout);
            assert!(
                o.status.success(),
                "{name} {tier} {threads}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            let got: Vec<&str> = out
                .lines()
                .filter_map(|l| l.find("RES ").map(|at| &l[at..]))
                .collect();
            assert_eq!(got, want, "{name} {tier} {threads}: {out}");
        }
    }
}

#[test]
fn an_alias_of_a_constant_in_instance_data_reads_the_constant() {
    let tmp = tempdir("alias_constant");
    let work = analyse(&tmp, "design", "--std=2008", DESIGN);
    run_ok(&work, "--std=2008", "design", EXPECTED);
}

#[test]
fn an_alias_of_a_generic_in_an_elaboration_time_function_in_1993() {
    let tmp = tempdir("alias_constant_93");
    let work = analyse(&tmp, "design93", "--std=93", DESIGN_93);
    run_ok(&work, "--std=93", "design93", &["RES 123-1"]);
}

#[test]
fn an_alias_subtype_of_another_length_than_the_constant_is_an_error_at_the_alias() {
    let tmp = tempdir("alias_constant_bad");
    let at = MISMATCH.find("@@").expect("marker");
    let line = MISMATCH[..at].matches('\n').count() + 1;
    let column = at - MISMATCH[..at].rfind('\n').expect("line");
    let work = analyse(
        &tmp,
        "mismatch",
        "--std=2008",
        &MISMATCH.replacen("@@", "", 1),
    );
    for tier in tiers() {
        let o = yavs(&["run", "--std=2008", &work, tier, "--threads=1", "top"]);
        let e = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{tier}: {e}");
        for want in [
            "error[E3009]: value length 3 does not match x length 2",
            &format!("case.vhd:{line}:{column}"),
        ] {
            assert!(e.contains(want), "{tier}: {want}: {e}");
        }
        assert!(
            !String::from_utf8_lossy(&o.stdout).contains("RES"),
            "{tier}"
        );
    }
}
