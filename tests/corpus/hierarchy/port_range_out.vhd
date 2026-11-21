-- difftest: top=port_range_out std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives 10 into the actual with no check, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). The driving value of an out port is the driving
-- value of its actual and belongs to the actual's subtype (LRM 14.7.3, #1183): 5 goes
-- through, 10 does not fit `integer range 0 to 9`.
entity port_range_out_src is
  port (o : out integer := 0);
end entity;
architecture a of port_range_out_src is
begin
  process
  begin
    o <= 5;
    wait for 1 ns;
    o <= 10;
    wait;
  end process;
end architecture;

entity port_range_out is end entity;
architecture a of port_range_out is
  signal s : integer range 0 to 9 := 0;
begin
  u : entity work.port_range_out_src port map (o => s);
  process
  begin
    wait for 0 ns;
    report "s=" & integer'image(s);
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
