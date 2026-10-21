-- Four signals that all change in the same cycles (#496): a callback on one registers a
-- callback on another that the simulator reports before or after it.
library ieee;
use ieee.std_logic_1164.all;

entity lateedge is end entity;

architecture sim of lateedge is
  signal fa, fb, fc, fd : std_ulogic := '0';
begin
  p : process begin
    wait for 10 ns;
    (fa, fb, fc, fd) <= std_ulogic_vector'("1111");
    wait for 20 ns;
    (fa, fb, fc, fd) <= std_ulogic_vector'("0000");
    wait for 20 ns;
    (fa, fb, fc, fd) <= std_ulogic_vector'("1111");
    wait;
  end process;
end architecture;
