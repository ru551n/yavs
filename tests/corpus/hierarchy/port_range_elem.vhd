-- difftest: top=port_range_elem std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives 12 into the element with no check, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). An out port on an element of an array of a narrower
-- element subtype: its driving value belongs to the element subtype (LRM 14.7.3, #1183).
entity port_range_elem_src is
  port (o : out integer := 0);
end entity;
architecture a of port_range_elem_src is
begin
  process
  begin
    o <= 9;
    wait for 1 ns;
    o <= 12;
    wait;
  end process;
end architecture;

entity port_range_elem is end entity;
architecture a of port_range_elem is
  type digits is array (0 to 2) of integer range 0 to 9;
  signal s : digits := (others => 0);
begin
  u : entity work.port_range_elem_src port map (o => s(1));
  process
  begin
    wait for 0 ns;
    report "s(1)=" & integer'image(s(1));
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
