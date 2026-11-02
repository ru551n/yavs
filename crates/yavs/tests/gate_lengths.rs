//! The result length of a gate of two vectors (`and`..`xnor`, `not`, `to_x01`) is a fact the
//! optimiser folds the target's length check with only when the operand lengths are proven equal
//! (#1063). A target of another length, operands of different lengths, and lengths a generic
//! decides per instance still fail at the statement, with the same message, in every tier. A gate
//! of equal known lengths is a `LogicOp` (no native call): every element equals the scalar gate.

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity gl_short is end;
architecture a of gl_short is begin process
  variable a, b : std_ulogic_vector(7 downto 0) := x"5a"; variable t : std_ulogic_vector(3 downto 0);
begin t := a xor b; wait; end process; end;
library ieee; use ieee.std_logic_1164.all;
entity gl_operands is end;
architecture a of gl_operands is begin process
  variable a : std_ulogic_vector(7 downto 0) := x"5a"; variable c : std_ulogic_vector(3 downto 0) := x"3";
  variable t : std_ulogic_vector(7 downto 0);
begin t := a xor c; wait; end process; end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity gl_num is end;
architecture a of gl_num is begin process
  variable a : unsigned(7 downto 0) := x"5a"; variable c : unsigned(3 downto 0) := x"3"; variable t : unsigned(7 downto 0);
begin t := a and c; wait; end process; end;
library ieee; use ieee.std_logic_1164.all;
entity gl_not is end;
architecture a of gl_not is begin process
  variable a : std_ulogic_vector(7 downto 0) := x"5a"; variable t : std_ulogic_vector(8 downto 0);
begin t := to_x01(not a); wait; end process; end;
library ieee; use ieee.std_logic_1164.all;
entity gl_gen_u is generic (W : positive); end;
architecture a of gl_gen_u is
  signal clk : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk)
    variable a, b : std_ulogic_vector(W - 1 downto 0) := (others => '1');
    variable t : std_ulogic_vector(7 downto 0); variable n : natural := 0;
  begin
    if rising_edge(clk) then n := n + 1; if n > 4 then t := a xnor b; end if; end if;
  end process;
end;
entity gl_gen is end;
architecture a of gl_gen is begin
  ok: entity work.gl_gen_u generic map (W => 8);
  bad: entity work.gl_gen_u generic map (W => 9);
end;
library ieee; use ieee.std_logic_1164.all;
entity gl_table is end;
architecture a of gl_table is
  signal clk : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  -- Every pair of the nine values, in one block (9 elements) and over two (81): each element of a
  -- vector gate equals the scalar gate of its elements.
  process (clk)
    constant V : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
    variable a, b, r : std_ulogic_vector(0 to 80);
    variable s, t, q : std_ulogic_vector(0 to 8);
    variable bv : bit_vector(0 to 9) := "0110100110";
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      for i in 0 to 80 loop a(i) := V(i / 9); b(i) := V(i mod 9); end loop;
      s := V; t := V(4 to 8) & V(0 to 3);
      r := a and b; for i in r'range loop assert r(i) = (a(i) and b(i)) report "and" severity failure; end loop;
      r := a nand b; for i in r'range loop assert r(i) = (a(i) nand b(i)) report "nand" severity failure; end loop;
      r := a or b; for i in r'range loop assert r(i) = (a(i) or b(i)) report "or" severity failure; end loop;
      r := a nor b; for i in r'range loop assert r(i) = (a(i) nor b(i)) report "nor" severity failure; end loop;
      r := a xor b; for i in r'range loop assert r(i) = (a(i) xor b(i)) report "xor" severity failure; end loop;
      r := a xnor b; for i in r'range loop assert r(i) = (a(i) xnor b(i)) report "xnor" severity failure; end loop;
      r := not a; for i in r'range loop assert r(i) = not a(i) report "not" severity failure; end loop;
      r := to_x01(b); for i in r'range loop assert r(i) = to_x01(b(i)) report "to_x01" severity failure; end loop;
      r := to_x01z(b); for i in r'range loop assert r(i) = to_x01z(b(i)) report "to_x01z" severity failure; end loop;
      r := to_ux01(b); for i in r'range loop assert r(i) = to_ux01(b(i)) report "to_ux01" severity failure; end loop;
      q := s xor t; for i in q'range loop assert q(i) = (s(i) xor t(i)) report "xor 9" severity failure; end loop;
      q := s nor t; for i in q'range loop assert q(i) = (s(i) nor t(i)) report "nor 9" severity failure; end loop;
      q := not t; for i in q'range loop assert q(i) = not t(i) report "not 9" severity failure; end loop;
      -- A `not` of the lowering's own (an implicit operator), whose second operand is no array.
      bv := not bv; assert bv = "1001011001" report "bit not" severity failure; bv := not bv;
      if n = 3 then report "table ok"; std.env.finish; end if;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity gl_ok is end;
architecture a of gl_ok is begin process
  variable a : std_ulogic_vector(7 downto 0) := "01XZ10HL"; variable b : std_ulogic_vector(7 downto 0) := "1W0-U1LH";
  variable u : unsigned(7 downto 0) := x"a5"; variable t : std_ulogic_vector(7 downto 0);
begin
  t := a xor b; report to_string(t);
  t := a nand b; report to_string(t);
  t := not a; report to_string(t);
  t := to_ux01(b); report to_string(t);
  u := u xor x"0f"; report to_string(u);
  wait;
end process; end;
"#;

/// (top, what stderr must say, line of the failing statement).
const CASES: &[(&str, &str, usize)] = &[
    (
        "gl_short",
        "error[E3009]: value length 8 does not match variable T length 4",
        5,
    ),
    (
        "gl_operands",
        "arguments of overloaded 'xor' operator are not of the same length",
        0,
    ),
    (
        "gl_num",
        "arguments of overloaded 'and' operator are not of the same length",
        0,
    ),
    (
        "gl_not",
        "error[E3009]: value length 8 does not match variable T length 9",
        21,
    ),
    (
        "gl_gen",
        "error[E3009]: value length 9 does not match variable T length 8",
        32,
    ),
];

#[test]
fn gate_lengths_still_fail_and_equal_ones_compute() {
    let mut tiers: Vec<&[&str]> = vec![&["--tier=interp"]];
    if cfg!(feature = "clift") {
        tiers.push(&["--tier=clift"]);
        tiers.push(&["--tier=auto", "--tier-heat=1,0,0"]);
    }
    let tmp = tempdir("gate_lengths");
    let file = tmp.join("g.vhd");
    std::fs::write(&file, SRC).expect("write");
    let work = format!("--work=work:{}/lib", tmp.display());
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run")
    };
    let o = yavs(&["analyze", "--std=2008", &work, file.to_str().expect("path")]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let mut good = None;
    for tier in tiers {
        let run = |top: &str| {
            let mut a = vec!["run", &work, "--threads=1", "--stop-time=200ns"];
            a.extend_from_slice(tier);
            a.push(top);
            yavs(&a)
        };
        for &(top, what, line) in CASES {
            let o = run(top);
            let e = String::from_utf8_lossy(&o.stderr) + String::from_utf8_lossy(&o.stdout);
            assert_eq!(o.status.code(), Some(1), "{top} {tier:?}: {e}");
            assert!(e.contains(what), "{top} {tier:?}: {e}");
            if line > 0 {
                assert!(e.contains(&format!("g.vhd:{line}:")), "{top} {tier:?}: {e}");
            }
        }
        let o = run("gl_table");
        let e =
            String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
        assert_eq!(o.status.code(), Some(0), "gl_table {tier:?}: {e}");
        assert!(e.contains("table ok"), "gl_table {tier:?}: {e}");
        let o = run("gl_ok");
        let e =
            String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
        assert_eq!(o.status.code(), Some(0), "gl_ok {tier:?}: {e}");
        assert!(e.contains("Note: 0fs+0: 1XXXU111"), "gl_ok {tier:?}: {e}");
        match &good {
            None => good = Some(e),
            Some(g) => assert_eq!(g, &e, "gl_ok {tier:?}"),
        }
    }
}
