-- The types and functions shared by the design
library ieee;
use ieee.std_logic_1164.all;

package counter_pkg is
  function all_ones(width : positive) return std_logic_vector;
end package;

package body counter_pkg is
  function all_ones(width : positive) return std_logic_vector is
  begin
    return (width - 1 downto 0 => '1');
  end function;
end package body;
