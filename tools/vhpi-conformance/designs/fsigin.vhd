-- A foreign procedure (VHPI model of plugins/fparam.c) with a signal parameter of mode in.
library ieee;
use ieee.std_logic_1164.all;

entity fp_sigin is end entity;

architecture sim of fp_sigin is
  procedure ones(signal s : in std_logic_vector; r : out integer);
  attribute foreign of ones : procedure is "VHPI fparam ones";
  procedure ones(signal s : in std_logic_vector; r : out integer) is begin end procedure;
  signal s : std_logic_vector(3 downto 0) := "0110";
  signal s_f : integer := -9;
begin
  process
    variable r : integer;
  begin
    wait for 1 ns;
    ones(s, r);
    s_f <= r;
    s <= "1110";
    wait for 1 ns;
    ones(s, r);
    s_f <= r;
    wait;
  end process;
end architecture;
