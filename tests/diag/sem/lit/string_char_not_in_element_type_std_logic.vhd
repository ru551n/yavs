-- yavs-test: std=2008
-- expect: E1330 7:47 character `2` is not a literal of type `std_ulogic`
-- LRM 15.7
library ieee;
use ieee.std_logic_1164.all;
package p is
  constant c : std_logic_vector(0 to 2) := "102";
end package;
