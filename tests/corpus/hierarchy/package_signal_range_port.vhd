-- difftest: top=package_signal_range_port std=2008
-- 'range of a constrained signal declared in a package is static: it sizes a port of an
-- entity and of a component (#294).
library ieee;
use ieee.std_logic_1164.all;
package sigs is
  signal gIntReq : std_logic_vector(7 downto 0) := (others => '0');
end package;
library ieee;
use ieee.std_logic_1164.all;
use work.sigs.all;
entity ir is
  port (IntReq : in std_logic_vector(gIntReq'range));
end entity;
architecture a of ir is begin
  process begin
    wait for 1 ns;
    assert IntReq'length = 8 and IntReq'left = 7 and IntReq = "10101010" severity failure;
    report "len " & integer'image(IntReq'length);
    wait;
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
entity package_signal_range_port is end entity;
architecture t of package_signal_range_port is
  use work.sigs.all;
  component ir is
    port (IntReq : in std_logic_vector(gIntReq'range));
  end component;
  constant n : integer := gIntReq'length;
  signal t : std_logic_vector(gIntReq'high downto gIntReq'low) := (others => '0');
  signal s : std_logic_vector(7 downto 0) := "10101010";
begin
  u : ir port map (IntReq => s);
  process begin
    assert n = 8 and t'length = 8 severity failure;
    wait;
  end process;
end architecture;
