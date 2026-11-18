-- Forwarders in front of procedures that wait, as a message library has them: one passes a signal
-- and builds a one-element array, one tests 'active before it calls on. They are inlined
-- (`opt/inline.rs` `waits_last`): the waiting procedure is the call.
package wait_pkg is
  type ints is array (natural range <>) of integer;
  procedure send(signal n : inout boolean; constant v : in integer; variable acc : inout integer);
end package;
package body wait_pkg is
  procedure notify(signal n : inout boolean) is
  begin
    n <= not n;
    wait for 0 ns;
  end procedure;
  procedure notify_net(signal n : inout boolean) is
  begin
    if not n'active then
      notify(n);
    end if;
  end procedure;
  procedure deliver(signal n : inout boolean; constant vs : in ints; variable acc : inout integer) is
  begin
    for i in vs'range loop
      acc := acc + vs(i);
    end loop;
    wait for 1 ns;
    notify_net(n);
  end procedure;
  procedure send(signal n : inout boolean; constant v : in integer; variable acc : inout integer) is
  begin
    deliver(n, ints'(0 => v), acc);
  end procedure;
end package body;

use work.wait_pkg.all;
entity wait_calls is
end entity;
architecture a of wait_calls is
  signal net : boolean := false;
begin
  process
    variable acc : integer := 0;
  begin
    for i in 1 to 50 loop
      send(net, i, acc);
    end loop;
    report "acc=" & integer'image(acc) & " at " & time'image(now);
    wait;
  end process;
end architecture;
