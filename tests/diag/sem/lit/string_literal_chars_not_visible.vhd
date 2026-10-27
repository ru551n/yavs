-- yavs-test: std=2008
-- expect: E1100 9:40 the character literal '0' of `STD_ULOGIC` is not visible here
-- expect: E1100 10:40 the character literal '1' of `STD_ULOGIC` is not visible here
-- help: use ieee.std_logic_1164.all;
-- LRM 9.3.2, 12.3
library ieee;
use ieee.numeric_std.all;
package p is
  constant a : unsigned(1 downto 0) := "01";
  constant b : unsigned(3 downto 0) := x"F";
end package;
