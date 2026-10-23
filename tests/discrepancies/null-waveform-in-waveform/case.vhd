-- A null element among other waveform elements of a bus assignment (LRM 10.5.2.2): the value
-- before it is applied, the null disconnects at its time, the value after it reconnects.
library ieee;
use ieee.std_logic_1164.all;

entity nullwave is end entity;

architecture test of nullwave is
  signal first, middle : std_logic bus;
begin
  process
  begin
    first <= '1', null after 2 ns;
    middle <= '1' after 1 ns, null after 3 ns, '0' after 5 ns;
    wait for 1 ns;
    report "RES 1 ns " & std_logic'image(first) & std_logic'image(middle);
    wait for 3 ns;
    report "RES 4 ns " & std_logic'image(first) & std_logic'image(middle);
    wait for 2 ns;
    report "RES 6 ns " & std_logic'image(first) & std_logic'image(middle);
    wait;
  end process;
end architecture;
