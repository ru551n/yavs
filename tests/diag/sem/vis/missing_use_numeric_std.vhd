-- yavs-test: std=2008
-- expect: E1100 7:14 `unsigned` is not declared
-- help: `unsigned` is declared in `ieee.numeric_std`; add `use ieee.numeric_std.all;`
-- LRM 12.4
library ieee;
package p is
  signal s : unsigned(3 downto 0);
end package;
