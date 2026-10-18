-- yavs-test: std=2008
-- expect: E1300 13:8 expected `unsigned`, found `std_logic_vector`
-- help: use `unsigned(x)` to convert
-- LRM 10.6.2
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity e is end entity;
architecture a of e is
  signal u : unsigned(3 downto 0);
  signal v : std_logic_vector(3 downto 0);
begin
  u <= v;
end architecture;
