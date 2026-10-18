-- yavs-test: std=2008
-- expect: E1812 9:21 unresolved signal `s` has multiple sources
-- LRM 14.7.2
entity e is end entity;
architecture a of e is
  signal s, t : bit_vector(0 to 3);
begin
  p1: process begin s(1 to 2) <= "11"; t(0 to 1) <= "00"; wait; end process;
  p2: process begin s(2) <= '1'; wait; end process;
  t(2 to 3) <= "11";
end architecture;
