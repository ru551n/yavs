-- difftest: top=mode_view_signal_parameter std=2019
-- 2019 signal parameters with a mode view (6.5.2): a procedure drives the `out` elements
-- of a view port and reads its `in` element.
package mode_view_signal_parameter_pkg is
  type bus_t is record valid : bit; data : integer; ready : bit; end record;
  view bus_m of bus_t is valid, data : out; ready : in; end view;
  alias bus_s is bus_m'converse;
  procedure send (signal b : view bus_m; d : integer);
end package;
package body mode_view_signal_parameter_pkg is
  procedure send (signal b : view bus_m; d : integer) is
  begin
    b.data <= d;
    b.valid <= '1';
    wait until b.ready = '1';
    b.valid <= '0';
  end procedure;
end package body;

use work.mode_view_signal_parameter_pkg.all;
entity mode_view_signal_parameter_m is port (b : view bus_m); end;
architecture a of mode_view_signal_parameter_m is begin
  process begin
    send(b, 3);
    wait for 5 ns;
    send(b, 4);
    report "sent";
    wait;
  end process;
end;

use work.mode_view_signal_parameter_pkg.all;
entity mode_view_signal_parameter is end;
architecture t of mode_view_signal_parameter is
  signal s : bus_t := ('0', 0, '0');
begin
  m : entity work.mode_view_signal_parameter_m port map (b => s);
  process begin
    wait until s.valid = '1';
    report "got " & integer'image(s.data);
    s.ready <= '1', '0' after 1 ns;
    wait until s.valid = '0';
    wait until s.valid = '1';
    report "got " & integer'image(s.data);
    s.ready <= '1', '0' after 1 ns;
    wait;
  end process;
end;
