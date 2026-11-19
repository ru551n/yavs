-- A `numeric_std` shift function given a negative count known during elaboration (generic `k`,
-- `W` replaced by a width): the normal mode stops at the call, so the process stays out and both
-- modes stop with one error (cycle mode shifted the other way and ran on).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shiftneg is
  generic (k : integer := -1);
end entity;
architecture t of cycle_shiftneg is
  signal clk : std_ulogic := '0';
  signal x : unsigned(W - 1 downto 0) := (others => '1');
  signal q, r : unsigned(W - 1 downto 0);
begin
  clk <= not clk after 5 ns when now < 50 ns;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= shift_left(x, k);
      r <= rotate_right(x, k);
    end if;
  end process;
  main : process
  begin
    wait for 12 ns;
    report "q=" & to_hstring(q) & " r=" & to_hstring(r);
    wait;
  end process;
end architecture;
