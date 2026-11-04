-- difftest: top=wide_history_ext
-- A wide bus read through an external name: nothing in the design can be traced, so every wide
-- signal keeps its history.
library ieee;
use ieee.std_logic_1164.all;

entity wide_history_ext is
end entity;

architecture t of wide_history_ext is
  subtype bus_t is std_logic_vector(99 downto 0);
  signal viaext, other : bus_t := (others => '0');
begin
  process (viaext)
    alias ex is << signal .wide_history_ext.viaext : bus_t >>;
  begin
    report "external event " & boolean'image(ex'event) & " last event " & time'image(ex'last_event);
  end process;

  process (other)
  begin
    report "other " & std_logic'image(other(70));
  end process;

  process
  begin
    wait for 5 ns;
    viaext <= (others => '1');
    other <= (others => '1');
    wait for 10 ns;
    viaext <= (70 => '1', others => '0');
    other <= (70 => '1', others => '0');
    wait for 10 ns;
    report "done";
    wait;
  end process;
end architecture;
