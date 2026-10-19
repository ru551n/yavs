-- difftest: top=port_length_const std=2008
-- An architecture constant computed from an unconstrained port's 'length sees the actual's
-- bounds (#248).
library ieee;
use ieee.std_logic_1164.all;
entity port_length_const_dut is
  port (wdata : in std_logic_vector);
end;
architecture a of port_length_const_dut is
  constant n : natural := wdata'length;
  constant m : natural := 256 * wdata'length;
begin
  process begin
    report "n=" & integer'image(n) & " m=" & integer'image(m) & " len="
      & integer'image(wdata'length);
    assert n = 32 and m = 8192 severity failure;
    wait;
  end process;
end;
library ieee;
use ieee.std_logic_1164.all;
entity port_length_const is end;
architecture a of port_length_const is
  signal w : std_logic_vector(31 downto 0);
begin
  dut : entity work.port_length_const_dut port map (wdata => w);
end;
