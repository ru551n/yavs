-- difftest: top=port_range_init std=2008 exit=2 ref=hand
-- ref=hand: sim1 starts the actual at INTEGER'LEFT with no check, sim2 rejects the association
-- at elaboration (discrepancy `port-range`). An out port with no default starts at its
-- subtype's 'LEFT, and that is the driving value its actual starts from (LRM 14.7.5.2): it is
-- checked against the actual's subtype like every later value (#1183).
entity port_range_init_src is
  port (o : out integer);
end entity;
architecture a of port_range_init_src is
begin
  process
  begin
    wait for 1 ns;
    o <= 1;
    wait;
  end process;
end architecture;

entity port_range_init is end entity;
architecture a of port_range_init is
  signal s : natural;
begin
  u : entity work.port_range_init_src port map (o => s);
  process
  begin
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
