-- difftest: top=port_range_real std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives 1.5 into the actual with no check, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). A floating-point out port on a narrower actual
-- (LRM 14.7.3, #1183).
entity port_range_real_src is
  port (r : out real := 0.0);
end entity;
architecture a of port_range_real_src is
begin
  process
  begin
    wait for 1 ns;
    r <= 1.5;
    wait;
  end process;
end architecture;

entity port_range_real is end entity;
architecture a of port_range_real is
  subtype unit is real range 0.0 to 1.0;
  signal u : unit := 0.0;
begin
  i : entity work.port_range_real_src port map (r => u);
  process
  begin
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
