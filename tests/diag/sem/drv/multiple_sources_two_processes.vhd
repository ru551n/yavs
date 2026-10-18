-- yavs-test: std=2008
-- expect: E1812 9:21 unresolved signal `s` has multiple sources
-- LRM 14.7.2, 6.4.2.3
entity e is end entity;
architecture a of e is
  signal s : bit;
begin
  s <= '1';
  p2: process begin s <= '0'; wait; end process;
end architecture;
