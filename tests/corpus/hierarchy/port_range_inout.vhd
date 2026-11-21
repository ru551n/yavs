-- difftest: top=port_range_inout std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives -1 into the actual with no check, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). The driving value of an inout port is a source of
-- its actual and belongs to the actual's subtype (LRM 14.7.3, #1183): 4 goes through, -1 does
-- not fit `natural`.
entity port_range_inout_src is
  port (b : inout integer := 0);
end entity;
architecture a of port_range_inout_src is
begin
  process
  begin
    b <= 4;
    wait for 1 ns;
    report "b=" & integer'image(b);
    b <= -1;
    wait;
  end process;
end architecture;

entity port_range_inout is end entity;
architecture a of port_range_inout is
  signal s : natural := 0;
begin
  u : entity work.port_range_inout_src port map (b => s);
  process
  begin
    wait for 0 ns;
    report "s=" & integer'image(s);
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
