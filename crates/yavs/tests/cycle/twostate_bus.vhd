-- `--two-state` (A71): a 'Z' driven onto a resolved signal stays 'Z' below the top.
-- `ev` is an event net as a verification library's message passing has one: a package drives
-- its idle value "ZZ" from the processes of components below the top. `bus` is a tri-state bus
-- two components below the top drive in turn. `od` is an open-drain line with a pull-up below
-- the top: an 'H' driven there is a strong '1' under `--two-state` (A65), so pulling it low
-- conflicts.
library ieee;
use ieee.std_logic_1164.all;

package twostate_ev is
  constant idle : std_logic_vector := "ZZ";
  function is_active(signal e : std_logic_vector) return boolean;
  procedure notify(signal e : inout std_logic_vector);
end package;

package body twostate_ev is
  function is_active(signal e : std_logic_vector) return boolean is
  begin
    return e /= idle;
  end function;

  procedure notify(signal e : inout std_logic_vector) is
  begin
    if e(e'left) = '1' then
      e(e'right) <= '1';
    else
      e(e'left) <= '1';
    end if;
    wait until is_active(e);
    e <= idle;
    wait for 0 ns;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.twostate_ev.all;

entity twostate_vc is
  generic (first : time);
  port (ev : inout std_logic_vector(0 to 1) := "ZZ");
end entity;

architecture a of twostate_vc is
begin
  process
  begin
    wait for first;
    for i in 1 to 3 loop
      notify(ev);
      wait for 10 ns;
    end loop;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity twostate_drv is
  generic (id : natural);
  port (turn : in natural; data : in std_logic_vector(7 downto 0); bus_io : inout std_logic_vector(7 downto 0));
end entity;

architecture a of twostate_drv is
begin
  bus_io <= data when turn = id else (others => 'Z');
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity twostate_pull is
  port (od : inout std_logic);
end entity;

architecture a of twostate_pull is
begin
  od <= 'H';
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity twostate_od is
  port (low : in boolean; od : inout std_logic);
end entity;

architecture a of twostate_od is
begin
  od <= '0' when low else 'Z';
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.twostate_ev.all;

entity twostate_bus is
end entity;

architecture tb of twostate_bus is
  signal ev : std_logic_vector(0 to 1) := "ZZ";
  signal turn : natural := 0;
  signal bus_io : std_logic_vector(7 downto 0);
  signal od : std_logic;
  signal low : boolean := false;
  constant d1 : std_logic_vector(7 downto 0) := x"A5";
  constant d2 : std_logic_vector(7 downto 0) := x"3C";
begin
  vc1 : entity work.twostate_vc generic map (first => 1 ns) port map (ev);
  vc2 : entity work.twostate_vc generic map (first => 6 ns) port map (ev);
  drv1 : entity work.twostate_drv generic map (1) port map (turn, d1, bus_io);
  drv2 : entity work.twostate_drv generic map (2) port map (turn, d2, bus_io);
  pull : entity work.twostate_pull port map (od);
  odrv : entity work.twostate_od port map (low, od);

  process
    variable n : natural := 0;
  begin
    while n < 6 loop
      wait until is_active(ev) for 100 ns;
      exit when not is_active(ev);
      n := n + 1;
      wait until not is_active(ev);
    end loop;
    report "events=" & integer'image(n) & " ev=" & to_string(ev);
    for t in 0 to 2 loop
      turn <= t;
      wait for 1 ns;
      report "turn=" & integer'image(t) & " bus=" & to_string(bus_io);
    end loop;
    report "pull=" & to_string(od);
    low <= true;
    wait for 1 ns;
    report "low=" & to_string(od);
    wait;
  end process;
end architecture;
