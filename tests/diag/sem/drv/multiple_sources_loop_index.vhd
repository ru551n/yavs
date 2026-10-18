-- yavs-test: std=2008
-- expect: E1812 9:21 unresolved signal `s` has multiple sources
-- LRM 8.1 (a loop parameter is not static: the driver is all of `s`)
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 3);
begin
  p1: process begin for i in 0 to 1 loop s(i) <= '1'; end loop; wait; end process;
  p2: process begin s(3) <= '1'; wait; end process;
end architecture;
