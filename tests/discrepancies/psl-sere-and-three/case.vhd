-- Three booleans joined by the length-matching and of a sequence.
library ieee;
use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal clk : std_logic := '0';
  signal x : std_logic := '1';
  signal y : std_logic := '0';
  signal z : bit := '1';
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 10 ns;
  d1: cover {x = '1' && y = '0' && z = '1'} report "RES three";
end architecture;
