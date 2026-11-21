-- difftest: top=port_range_init_in std=2008 exit=2 ref=hand
-- ref=hand: sim1 lets the port start at INTEGER'LEFT with no check, sim2 rejects the
-- association at elaboration (discrepancy `port-range`). An `integer` signal with no initial
-- value starts at INTEGER'LEFT, and that is the effective value its `natural` in port starts
-- from (LRM 14.7.3.3, 14.7.5.2): it is checked like every later value (#1183).
entity port_range_init_in_dst is
  port (i : in natural);
end entity;
architecture a of port_range_init_in_dst is
begin
  process (i)
  begin
    report "i=" & integer'image(i);
  end process;
end architecture;

entity port_range_init_in is end entity;
architecture a of port_range_init_in is
  signal s : integer;
begin
  u : entity work.port_range_init_in_dst port map (i => s);
  process
  begin
    s <= 1;
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
