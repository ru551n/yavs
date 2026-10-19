-- difftest: top=mode_view_param_procs std=2019
-- 2019 signal parameters with mode views (6.5.2) called from several processes: each drives
-- the view's `out` elements of its own element of an array signal, one through a procedure
-- declared in the process, one passing its view parameter on; a process per element drives
-- its `in` element.
package mode_view_param_procs_pkg is
  type bus_t is record valid : bit; data : integer; ready : bit; end record;
  view bus_m of bus_t is valid, data : out; ready : in; end view;
  type buses_t is array (0 to 2) of bus_t;
  procedure send (signal b : view bus_m; d : integer);
  procedure send_twice (signal b : view bus_m; d : integer);
end package;

package body mode_view_param_procs_pkg is
  procedure send (signal b : view bus_m; d : integer) is
  begin
    b.data <= d;
    b.valid <= '1';
    wait until b.ready = '1';
    b.valid <= '0';
    wait until b.ready = '0';
  end procedure;

  procedure send_twice (signal b : view bus_m; d : integer) is
  begin
    send(b, d);
    send(b, d + 1);
  end procedure;
end package body;

use work.mode_view_param_procs_pkg.all;
entity mode_view_param_procs is end;
architecture t of mode_view_param_procs is
  signal buses : buses_t;
begin
  p0 : process begin
    send(buses(0), 10);
    report "p0 done";
    wait;
  end process;

  p1 : process
    procedure local (signal b : view bus_m) is
    begin
      send(b, 20);
    end procedure;
  begin
    wait for 1 ns;
    local(buses(1));
    report "p1 done";
    wait;
  end process;

  p2 : process begin
    wait for 2 ns;
    send_twice(buses(2), 30);
    report "p2 done";
    wait;
  end process;

  ack : for i in buses'range generate
    process begin
      wait until buses(i).valid'event;
      if buses(i).valid = '1' then
        report "ack " & integer'image(i) & " data=" & integer'image(buses(i).data);
      end if;
      buses(i).ready <= buses(i).valid;
    end process;
  end generate;
end;
