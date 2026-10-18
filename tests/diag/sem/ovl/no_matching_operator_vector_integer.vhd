-- yavs-test: std=2008
-- expect: E1301 11:27 no operator `and` matches the operands
-- help: use `to_unsigned(x, n)` to convert an integer to a vector
-- LRM 9.2
library ieee;
use ieee.numeric_std.all;
package p is
  signal u : unsigned(3 downto 0);
  signal s : unsigned(3 downto 0);
  signal b : integer;
  constant c : boolean := u and b;
end package;
