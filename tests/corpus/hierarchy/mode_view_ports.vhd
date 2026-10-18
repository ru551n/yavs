-- difftest: top=mode_view_ports std=2019
-- 2019 mode views (per-element port modes, 'converse) and a linkage port (#75).
package mode_view_ports_pkg is
  type bus_t is record req : bit; addr : integer; ack : bit; end record;
  view master of bus_t is req, addr : out; ack : in; end view;
  alias slave is master'converse;
end;
use work.mode_view_ports_pkg.all;
entity mode_view_ports_m is port (b : view master; l : linkage bit); end;
architecture a of mode_view_ports_m is begin
  process begin
    b.req <= '1'; b.addr <= 5; wait until b.ack = '1';
    report "acked addr=" & integer'image(b.addr); wait;
  end process;
end;
use work.mode_view_ports_pkg.all;
entity mode_view_ports_s is port (b : view slave); end;
architecture a of mode_view_ports_s is begin
  process begin
    wait until b.req = '1';
    report "req addr=" & integer'image(b.addr);
    b.ack <= '1' after 1 ns; wait;
  end process;
end;
use work.mode_view_ports_pkg.all;
entity mode_view_ports is end;
architecture t of mode_view_ports is
  signal s : bus_t := ('0', 0, '0');
  signal x : bit;
begin
  m : entity work.mode_view_ports_m port map (s, x);
  sl : entity work.mode_view_ports_s port map (b => s);
end;
