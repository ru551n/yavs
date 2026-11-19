-- Cycle mode: an 'X' set by a process outside the region, passed on by a member's copy, read by
-- the testbench (the shape of a bus model's "got 0, expected X"). A documented loss: the copy
-- joins and the testbench reads '0' where the normal mode gives 'X'.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_xcopy is
end entity;
architecture a of cycle_xcopy is
  signal clk, con, pin : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 100 ns else clk;
  drv : process
  begin
    con <= '1';
    wait for 12 ns;
    con <= 'X';
    wait for 10 ns;
    con <= '0';
    wait;
  end process;
  copy : process (con)
  begin
    pin <= con;
  end process;
  stim : process
  begin
    wait for 15 ns;
    report "pin=" & std_logic'image(pin);
    wait for 10 ns;
    report "pin=" & std_logic'image(pin);
    wait;
  end process;
end architecture;
