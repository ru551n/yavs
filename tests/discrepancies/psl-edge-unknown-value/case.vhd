-- rose and fell of a std_logic signal that passes through 'X' in a clocked assertion: an edge needs a known value
-- on both sides.
library ieee;
use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal clk : std_logic := '0';
  signal x : std_logic := '0';
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 60 ns;
  x <= '1' after 12 ns, 'X' after 22 ns, '1' after 32 ns, '0' after 42 ns;
  r : assert never rose(x) report "RES rose" severity note;
  f : assert never fell(x) report "RES fell" severity note;
end architecture;
