-- difftest: top=port_range_in std=2008 exit=1 ref=hand
-- ref=hand: sim1 lets the port read 10, sim2 rejects the association at elaboration
-- (discrepancy `port-range`). The effective value of an in port is its actual's and belongs
-- to the port's subtype (LRM 14.7.3.3, #1183): 3 goes through, 10 does not fit
-- `integer range 0 to 9`.
entity port_range_in_dst is
  port (i : in integer range 0 to 9);
end entity;
architecture a of port_range_in_dst is
begin
  process (i)
  begin
    report "i=" & integer'image(i);
  end process;
end architecture;

entity port_range_in is end entity;
architecture a of port_range_in is
  signal s : integer := 0;
begin
  u : entity work.port_range_in_dst port map (i => s);
  process
  begin
    s <= 3;
    wait for 1 ns;
    s <= 10;
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
