-- Asserts and reports of pure functions whose calls fold with constants (A22): elaboration
-- evaluates them once per process; one of severity error or failure keeps its process in the
-- event kernel, which reports it when the call runs.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package cycle_foldrep_pkg is
  function half(n : natural; s : severity_level) return natural;
  -- A width check of the kind of an AXI package's: a message constant, reports only on failure.
  function width_ok(w : integer) return boolean;
  subtype size_t is unsigned(2 downto 0);
  function to_sz(w : positive) return size_t;
  function sz_bits(w : positive) return positive;
  -- A `while` loop the lowering does not take: only elaboration's value folds it.
  function log_sz(w : positive) return size_t;
end package;

package body cycle_foldrep_pkg is
  function half(n : natural; s : severity_level) return natural is
  begin
    assert n mod 2 = 0 report "half of odd " & integer'image(n) severity s;
    return n / 2;
  end function;
  function width_ok(w : integer) return boolean is
    constant msg : string := ". Got width=" & integer'image(w) & ".";
  begin
    if w <= 0 or w mod 8 /= 0 then
      report "bad width" & msg;
      return false;
    end if;
    return true;
  end function;
  function to_sz(w : positive) return size_t is
    constant result : size_t := to_unsigned(w / 8 - 1, 3);
  begin
    assert width_ok(w) report "Invalid width" severity failure;
    return result;
  end function;
  function log_sz(w : positive) return size_t is
    variable n, k : natural := 0;
  begin
    n := w;
    while n > 1 loop
      n := n / 2;
      k := k + 1;
    end loop;
    return to_unsigned(k, 3);
  end function;
  function sz_bits(w : positive) return positive is
  begin
    assert width_ok(w) report "Invalid width" severity failure;
    return w + 3;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_foldrep_pkg.all;

entity cycle_foldrep_dut is
  generic (
    g : natural; s : severity_level; loop_odd : boolean; dead : boolean := false;
    trunc : boolean := false);
  port (clk : in std_logic; a, b, c, d, r : out unsigned(7 downto 0));
end entity;

architecture rtl of cycle_foldrep_dut is
  -- Holds for every argument: no output in either mode.
  function small(n : natural) return natural is
  begin
    assert n < 1000 report "too big" severity failure;
    return n + 1;
  end function;
  function say(n : natural) return natural is
  begin
    report "say " & integer'image(n);
    return n;
  end function;
  -- A call inside a call: elaboration evaluates the outer one whole.
  function outer(n : natural) return natural is
  begin
    return half(n + 2, s) + 1;
  end function;
  -- Made once, by the elaboration of the normal mode, in both modes.
  constant k : natural := say(1);
  -- `pa`'s truncating call when `trunc`: its call cache entry is made outside cycle mode's folds.
  constant kt : unsigned(7 downto 0) := to_unsigned(300 * boolean'pos(trunc) + g, 8);
  -- An inlined call (a signal argument) folding an asserting call of its constant formal.
  function pack(x : unsigned; w : positive) return unsigned is
    variable r : unsigned(sz_bits(w) - 1 downto 0) := (others => '0');
  begin
    r(x'length - 1 downto 0) := x;
    return r(7 downto 0);
  end function;
  signal ra, rb, rc, rd : unsigned(7 downto 0) := (others => '0');
begin
  -- Two processes calling one function: each reports.
  pa : process (clk) begin
    if rising_edge(clk) then
      ra <= ra + half(g, s) + small(g);
      -- Never runs: elaboration's failure here is not seen in either mode.
      if dead then
        ra <= ra + half(1, failure);
      end if;
      -- A library warning of a folded call: made once too.
      if trunc then
        ra <= ra + to_unsigned(300 + g, 8);
      end if;
    end if;
  end process;
  pb : process (clk) begin
    if rising_edge(clk) then
      rb <= rb + outer(g) + k;
    end if;
  end process;
  -- The same call as `pa`'s: elaboration evaluates it again, from its call cache.
  pd : process (clk) begin
    if rising_edge(clk) then
      rd <= rd + half(g, s) + to_sz(8 * g) + pack(rd, 8 * g) + log_sz(8 * g);
    end if;
  end process;
  -- Arguments known only in the unrolled loop: elaboration does not evaluate these calls.
  pc : process (clk)
    variable acc : natural;
  begin
    if rising_edge(clk) then
      acc := 0;
      for i in 0 to 1 loop
        if loop_odd then
          acc := acc + half(g + i, s);
        else
          acc := acc + half(2 * (g + i), s);
        end if;
      end loop;
      rc <= rc + acc;
    end if;
  end process;
  a <= ra;
  b <= rb;
  c <= rc;
  d <= rd;
  r <= to_unsigned(say(g), 8);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_foldrep is
  generic (
    g : natural := 4; s : severity_level := warning; loop_odd : boolean := false;
    dead : boolean := false; trunc : boolean := false);
end entity;

architecture tb of cycle_foldrep is
  signal clk : std_logic := '0';
  signal a, b, c, d, r : unsigned(7 downto 0);
begin
  u : entity work.cycle_foldrep_dut generic map (g, s, loop_odd, dead, trunc) port map (clk, a, b, c, d, r);
  clk <= not clk after 5 ns when now < 40 ns;
  process begin
    wait for 42 ns;
    report "a=" & integer'image(to_integer(a)) & " b=" & integer'image(to_integer(b))
      & " c=" & integer'image(to_integer(c)) & " d=" & integer'image(to_integer(d))
      & " r=" & integer'image(to_integer(r));
    wait;
  end process;
end architecture;
