-- difftest: top=inout_port_collapse_external std=2008 ref=hand
-- A process that drives its `inout` formal and, through an external name, the formal's actual:
-- two drivers of one process, on the port and on the signal, which are one signal once the
-- port collapses into its actual. Both stay sources of the actual.
-- ref=hand: the first reference simulator crashes in the run and the second fails to elaborate
-- a drive through an external name in a process. Values by hand, `xa` resolving the formal's
-- driver, the external driver and the parent's driver (all start at 'Z'): 1 at 0 ns (1, Z, Z),
-- X at 2 ns (1, 0, Z), 0 at 4 ns (Z, 0, Z), still 0 at 5 ns (Z, 0, L), L at 6 ns (Z, Z, L).
library ieee;
use ieee.std_logic_1164.all;

entity iopp_ext is
  port (e : inout std_logic := 'Z');
end;
architecture t of iopp_ext is
begin
  drv : process
  begin
    e <= '1';
    wait for 2 ns;
    << signal .inout_port_collapse_external.xa : std_logic >> <= '0';
    wait for 2 ns;
    e <= 'Z';
    wait for 2 ns;
    << signal .inout_port_collapse_external.xa : std_logic >> <= 'Z';
    wait;
  end process;
  rd : process (e)
  begin
    report "ext e=" & to_string(e);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity inout_port_collapse_external is end;
architecture t of inout_port_collapse_external is
  signal xa : std_logic := 'Z';
begin
  ue : entity work.iopp_ext port map (e => xa);
  drv_xa : xa <= 'Z', 'L' after 5 ns;
  rd : process (xa)
  begin
    report "top xa=" & to_string(xa);
  end process;
  check : process
  begin
    wait for 9 ns;
    assert xa = 'L' report "xa=" & to_string(xa) severity failure;
    report "done";
    wait;
  end process;
end;
