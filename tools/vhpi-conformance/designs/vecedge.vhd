-- Value changes of a vector where several separate runs of elements change in one cycle, and a
-- scalar that changes in the same cycles (#346).
library ieee;
use ieee.std_logic_1164.all;

entity vecedge is end entity;

architecture sim of vecedge is
  signal v : std_ulogic_vector(7 downto 0) := (others => '0');
  signal w : std_ulogic := '0';
begin
  p : process begin
    wait for 10 ns;
    v <= "01011010";
    w <= '1';
    wait for 10 ns;
    v <= "00000000";
    w <= '0';
    wait for 10 ns;
    v <= "10000001";
    wait for 10 ns;
    v <= "10000001"; -- a transaction without a change
    wait for 10 ns;
    v <= "00000000";
    wait;
  end process;
end architecture;
