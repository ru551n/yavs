-- difftest: top=port_range_comp std=2008 exit=1 ref=hand
-- ref=hand: sim1 lets the ports read -1, sim2 rejects the association at elaboration
-- (discrepancy `port-range`). A component port is a port of its own: the value it receives
-- from its actual belongs to its subtype (LRM 14.7.3.3, #1183), whatever the entity's port.
entity port_range_comp_dst is
  port (i : in integer);
end entity;
architecture a of port_range_comp_dst is
begin
  process (i)
  begin
    report "i=" & integer'image(i);
  end process;
end architecture;

entity port_range_comp is end entity;
architecture a of port_range_comp is
  component port_range_comp_dst is
    port (i : in natural);
  end component;
  signal s : integer := 2;
begin
  u : port_range_comp_dst port map (i => s);
  process
  begin
    wait for 1 ns;
    s <= -1;
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
