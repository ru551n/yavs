-- yavs-test: std=2008
-- expect: E1300 12:8 expected `unsigned`, found `integer`
-- help: use `to_unsigned(x, n)` to convert
-- LRM 10.5.2
library ieee;
use ieee.numeric_std.all;
entity e is end entity;
architecture a of e is
  signal u : unsigned(3 downto 0);
  signal n : integer;
begin
  u <= n;
end architecture;
