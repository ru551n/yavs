-- difftest: top=port_range_buffer std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives -3 into the actual with no check, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). The driving value of a buffer port is the driving
-- value of its actual and belongs to the actual's subtype (LRM 14.7.3, #1183).
entity port_range_buffer_src is
  port (o : buffer integer := 1);
end entity;
architecture a of port_range_buffer_src is
begin
  process
  begin
    wait for 1 ns;
    o <= o - 4;
    wait;
  end process;
end architecture;

entity port_range_buffer is end entity;
architecture a of port_range_buffer is
  signal s : natural;
begin
  u : entity work.port_range_buffer_src port map (o => s);
  process
  begin
    wait for 0 ns;
    report "s=" & integer'image(s);
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
