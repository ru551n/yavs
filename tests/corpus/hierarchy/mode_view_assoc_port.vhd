-- difftest: top=mode_view_assoc_port std=2019 ignore=order
-- ignore=order: the processes that resume in one cycle report in a reference-specific order.
-- 2019 individual association of a port with a mode view (6.5.7.1, 6.5.2): the elements of the
-- port are connected one by one, the `out` elements drive their actuals, the `in` ones read.
package mode_view_assoc_port_pkg is
  type bus_t is record valid : bit; data : integer; ready : bit; end record;
  view bus_m of bus_t is valid, data : out; ready : in; end view;
  alias bus_s is bus_m'converse;
end package;

use work.mode_view_assoc_port_pkg.all;
entity mode_view_assoc_port_m is port (p : view bus_m); end;
architecture a of mode_view_assoc_port_m is begin
  process begin
    p.data <= 3;
    p.valid <= '1';
    wait until p.ready = '1';
    report "sent";
    p.valid <= '0';
    wait;
  end process;
end;

use work.mode_view_assoc_port_pkg.all;
entity mode_view_assoc_port_s is port (p : view bus_s); end;
architecture a of mode_view_assoc_port_s is begin
  process begin
    wait until p.valid = '1';
    report "got " & integer'image(p.data);
    p.ready <= '1', '0' after 1 ns;
    wait;
  end process;
end;

use work.mode_view_assoc_port_pkg.all;
entity mode_view_assoc_port is end;
architecture t of mode_view_assoc_port is
  signal v, r : bit;
  signal d : integer;
  signal s : bus_t;
begin
  m1 : entity work.mode_view_assoc_port_m port map (p.ready => r, p.data => d, p.valid => v);
  s1 : entity work.mode_view_assoc_port_s port map (p.valid => v, p.data => d, p.ready => r);
  m2 : entity work.mode_view_assoc_port_m port map (p.valid => s.valid, p.data => s.data, p.ready => s.ready);
  process begin
    wait until s.valid = '1';
    report "tb got " & integer'image(s.data);
    s.ready <= '1', '0' after 1 ns;
    wait;
  end process;
end;
