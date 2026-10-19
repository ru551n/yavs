-- A foreign architecture (VHPI model of plugins/farch.c) instantiated in a testbench: the
-- model creates a process and drivers for its out ports and drives them with
-- vhpi_schedule_transaction from value-change callbacks on its in port. The library word is
-- the plugin's file name, as one reference simulator loads it from there.
library ieee;
use ieee.std_logic_1164.all;

entity finv is
  port (
    a : in  std_logic_vector(3 downto 0);
    y : out std_logic_vector(3 downto 0);
    n : out integer;
    z : out bit
  );
end entity;

architecture model of finv is
  attribute foreign of model : architecture is "VHPI farch;inv";
begin
  -- Not elaborated: the foreign attribute wins.
  y <= "1010";
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity farch is end entity;

architecture sim of farch is
  signal a : std_logic_vector(3 downto 0) := "0000";
  signal y : std_logic_vector(3 downto 0);
  signal n : integer := -1;
  signal z : bit;
begin
  u : entity work.finv port map (a => a, y => y, n => n, z => z);
  process
  begin
    wait for 10 ns;
    a <= "0011";
    wait for 10 ns;
    a <= "0101";
    wait for 1 ns;
    a <= "0110";
    wait for 20 ns;
    wait;
  end process;
end architecture;
