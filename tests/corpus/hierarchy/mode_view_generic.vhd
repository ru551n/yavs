-- difftest: top=mode_view_generic std=2019
-- 2019 mode views declared in a generic package: each instance has its own views, and
-- 'converse of an instance's view (6.5.2).
package mode_view_generic_gp is
  generic (type data_t);
  type bus_t is record d : data_t; v : bit; ack : bit; end record;
  view src of bus_t is d, v : out; ack : in; end view;
  alias snk is src'converse;
end package;
package mode_view_generic_ip is new work.mode_view_generic_gp
  generic map (data_t => integer);

use work.mode_view_generic_ip.all;
entity mode_view_generic_m is port (b : view src); end;
architecture a of mode_view_generic_m is begin
  process begin
    b.d <= 42; b.v <= '1';
    wait until b.ack = '1';
    report "acked";
    wait;
  end process;
end;

use work.mode_view_generic_ip.all;
entity mode_view_generic_s is port (b : view snk); end;
architecture a of mode_view_generic_s is begin
  process begin
    wait until b.v = '1';
    report "d=" & integer'image(b.d);
    b.ack <= '1' after 2 ns;
    wait;
  end process;
end;

use work.mode_view_generic_ip.all;
entity mode_view_generic is end;
architecture t of mode_view_generic is
  signal s : bus_t := (0, '0', '0');
begin
  m : entity work.mode_view_generic_m port map (b => s);
  sl : entity work.mode_view_generic_s port map (b => s);
end;
