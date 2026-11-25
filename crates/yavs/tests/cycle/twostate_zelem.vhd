-- #1199: a 'Z' driven onto an element of a resolved vector in the two-state scope stays 'Z', as
-- one driven onto a whole resolved signal does: the testbench's values win on the line.
library ieee;
use ieee.std_logic_1164.all;

entity zelem_dut is
  port (
    o, oe : in    std_ulogic_vector(1 downto 0);
    pad   : inout std_logic_vector(2 downto 0)
  );
end entity;

architecture a of zelem_dut is
begin
  g : for k in 0 to 1 generate
    pad(k) <= o(k) when oe(k) = '1' else 'Z';
  end generate;
  pad(2) <= 'Z';
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity zelem_tb is
end entity;

architecture tb of zelem_tb is
  signal o, oe : std_ulogic_vector(1 downto 0) := "00";
  signal pad   : std_logic_vector(2 downto 0);
begin
  dut : entity work.zelem_dut port map (o, oe, pad);
  process
  begin
    pad <= "101";
    wait for 1 ns;
    report "pad=" & to_string(pad);
    oe <= "01";
    wait for 1 ns;
    report "pad=" & to_string(pad);
    wait;
  end process;
end architecture;
