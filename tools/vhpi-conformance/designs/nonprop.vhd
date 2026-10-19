-- Targets of the non-propagating put modes: two signals without drivers and a process that
-- counts their events.
library ieee;
use ieee.std_logic_1164.all;

entity nonprop is end entity;

architecture sim of nonprop is
  signal s   : std_logic_vector(3 downto 0) := "0000";
  signal i   : integer := 0;
  signal cnt : integer := 0;
begin
  mon : process (s, i) begin
    cnt <= cnt + 1;
  end process;
end architecture;
