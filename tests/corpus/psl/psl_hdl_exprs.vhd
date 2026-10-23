-- difftest: top=psl_hdl_exprs std=2008 exit=1 ignore=path
-- Conditions of the Boolean layer written as VHDL expressions of every kind of type it takes (IEEE 1850
-- clause 5.1): std_ulogic, bit, boolean, bit and std_logic vectors with slices, integers, reals,
-- strings, enumerations, record fields, function calls and numeric_std conversions. Each directive
-- fails once, at its own clock tick. The second reference simulator has no countones, so only the
-- first one blesses this test.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity psl_hdl_exprs is end;
architecture t of psl_hdl_exprs is
  type color is (red, green, blue);
  type pair is record f : integer; g : bit; end record;
  function twice (x : integer) return integer is begin return 2 * x; end function;
  signal clk : std_logic := '0';
  signal n : integer := 0;
  signal sl : std_ulogic := '0';
  signal bt : bit := '0';
  signal bo : boolean := false;
  signal v : std_logic_vector(3 downto 0) := "0000";
  signal bv : bit_vector(3 downto 0) := "0000";
  signal st : string(1 to 3) := "abc";
  signal r : real := 0.0;
  signal m : color := green;
  signal p : pair := (0, '0');
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      sl <= '1' when n = 1 else '0';
      bt <= '1' when n = 2 else '0';
      bo <= n = 3;
      st <= "bad" when n = 4 else "abc";
      r <= real(n) * 0.5;
      m <= red when n = 5 else green;
      v <= std_logic_vector(to_unsigned((n + 1) mod 16, 4));
      bv <= to_bitvector(std_logic_vector(to_unsigned((n + 1) mod 16, 4)));
      p <= (n, '1');
    end if;
  end process;
  e1: assert never sl report "std_ulogic";
  e2: assert never bt report "bit";
  e3: assert never bo report "boolean";
  e4: assert always (st /= "bad") report "string";
  e5: assert never (r = 3.0) report "real";
  e6: assert never (m = red) report "enumeration";
  e7: assert never (n > 6 and n < 10 and countones(v) = 1) report "onehot";
  e8: assert never (twice(n) = 18) report "function";
  e9: assert never (v = "1010") report "std_logic_vector";
  e10: assert never (p.f = 10 and p.g = '1') report "record";
  e11: assert never (to_integer(unsigned(v)) = 12) report "numeric_std";
  e12: assert never (bv(3 downto 2) = "11" and bv(1 downto 0) = "01") report "bit slice";
  e13: assert always (bv /= "1110" or n < 14) report "bit_vector";
  e14: assert always (v(3 downto 2) /= "11" or v(1 downto 0) /= "11" or n < 15) report "slice";
end;
