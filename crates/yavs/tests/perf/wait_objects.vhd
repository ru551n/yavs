-- A message procedure that makes an object and hands it to a procedure that waits is no forwarder
-- (no inlined body makes one): it is not optimised to find out (`opt/inline.rs` `ends_with_wait`).
-- `send` and `notify_net` are forwarders and are.
package wait_obj_pkg is
  type ints is array (natural range <>) of integer;
  type ints_ptr is access ints;
  procedure send(signal n : inout boolean; constant v : in integer; variable acc : inout integer);
  procedure post(signal n : inout boolean; constant v : in integer; variable acc : inout integer);
end package;
package body wait_obj_pkg is
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
  procedure post(signal n : inout boolean; constant v : in integer; variable acc : inout integer) is
    variable p : ints_ptr := new ints'(0 => v);
  begin
    deliver(n, p.all, acc);
  end procedure;
end package body;

use work.wait_obj_pkg.all;
entity wait_objects is
end entity;
architecture a of wait_objects is
  signal net : boolean := false;
begin
  process
    variable acc : integer := 0;
  begin
    for i in 1 to 50 loop
      if i mod 2 = 0 then
        post(net, i, acc);
      else
        send(net, i, acc);
      end if;
    end loop;
    report "acc=" & integer'image(acc) & " at " & time'image(now);
    wait;
  end process;
end architecture;
