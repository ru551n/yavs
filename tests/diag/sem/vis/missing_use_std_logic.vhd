-- yavs-test: std=2008
-- expect: E1100 7:14 `std_logic` is not declared
-- help: `std_logic` is declared in `ieee.std_logic_1164`; add `use ieee.std_logic_1164.all;`
-- LRM 12.4
library ieee;
package p is
  signal s : std_logic;
end package;
