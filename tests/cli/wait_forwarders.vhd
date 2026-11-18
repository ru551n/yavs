-- Forwarders in front of procedures that wait (`opt/inline.rs`, `waits_last`): a signal
-- parameter passed on, an aggregate built for the callee, a check before the call, a test of
-- 'active before it. Call paths, timeouts and a failure after a wait read as the calls.
use std.env.all;

package fw_pkg is
  type rec is record
    a, b : integer;
  end record;
  type rec_vec is array (natural range <>) of rec;
  procedure send(signal n : inout boolean; signal s : in integer; constant r : in rec;
                 variable acc : inout integer; constant timeout : in time := 5 ns);
  procedure send_checked(signal n : inout boolean; signal s : in integer; constant r : in rec;
                         variable acc : inout integer; constant bad : in integer);
end package;

package body fw_pkg is
  type ints is array (0 to 2) of integer;

  procedure chk(c : boolean; msg : string) is
  begin
    assert c report msg severity warning;
  end procedure;

  procedure notify(signal n : inout boolean) is
  begin
    n <= not n;
    wait for 0 ns;
  end procedure;

  -- Waits only in the call it ends with, after a test of the signal.
  procedure notify_net(signal n : inout boolean) is
  begin
    if not n'active then
      notify(n);
    end if;
  end procedure;

  procedure inner(signal n : inout boolean; signal s : in integer; constant rs : in rec_vec;
                  variable acc : inout integer; constant timeout : in time; constant bad : in integer) is
    constant tab : ints := (3, 5, 7);
  begin
    for i in rs'range loop
      acc := acc + rs(i).a * rs(i).b;
    end loop;
    wait on s for timeout;
    acc := acc + s + now / 1 ns;
    if acc mod 97 = 0 then
      report "note " & integer'image(acc) & " path " & to_string(get_call_path);
    end if;
    -- Out of range after the wait when `bad` is 3.
    acc := acc + tab((acc mod 2) + bad);
    notify_net(n);
  end procedure;

  -- Builds a one-element array for the callee, as a receive with one actor does.
  procedure send(signal n : inout boolean; signal s : in integer; constant r : in rec;
                 variable acc : inout integer; constant timeout : in time := 5 ns) is
  begin
    inner(n, s, rec_vec'(0 => r), acc, timeout, 0);
  end procedure;

  procedure send_checked(signal n : inout boolean; signal s : in integer; constant r : in rec;
                         variable acc : inout integer; constant bad : in integer) is
  begin
    chk(r.a >= 0, "negative a");
    inner(n, s, rec_vec'(0 => r), acc, 2 ns, bad);
  end procedure;
end package body;

use work.fw_pkg.all;

entity wait_forwarders is
  generic (bad : integer := 0);
end entity;

architecture a of wait_forwarders is
  signal net : boolean := false;
  signal s : integer := 0;
begin
  drive : process
  begin
    for i in 1 to 40 loop
      s <= i;
      wait for 3 ns;
    end loop;
    wait;
  end process;

  main : process
    variable acc : integer := 0;
  begin
    for i in 1 to 60 loop
      send(net, s, (i, i mod 7), acc);
      send_checked(net, s, (i - 30, 2), acc, 0);
    end loop;
    report "acc=" & integer'image(acc) & " at " & time'image(now);
    send_checked(net, s, (1, 2), acc, bad);
    report "done";
    wait;
  end process;
end architecture;
