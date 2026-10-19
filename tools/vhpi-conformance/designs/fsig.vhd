-- A foreign procedure (VHPI model of plugins/fparam.c) with signal parameters of mode in and
-- out. One reference simulator refuses out signal parameters at analysis, so the in-only case
-- is in fsigin.vhd.
library ieee;
use ieee.std_logic_1164.all;

entity fp_sig is end entity;

architecture sim of fp_sig is
  procedure sigp(signal s : in std_logic_vector; signal o : out std_logic_vector; signal k : out integer);
  attribute foreign of sigp : procedure is "VHPI fparam sigp";
  procedure sigp(signal s : in std_logic_vector; signal o : out std_logic_vector; signal k : out integer) is
  begin end procedure;
  signal s : std_logic_vector(3 downto 0) := "0110";
  signal o : std_logic_vector(3 downto 0) := "UUUU";
  signal k : integer := -9;
begin
  process
  begin
    wait for 1 ns;
    sigp(s, o, k);
    wait for 10 ns;
    s <= "1100";
    wait for 1 ns;
    sigp(s, o, k);
    wait;
  end process;
end architecture;
