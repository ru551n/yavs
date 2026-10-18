-- yavs-test: std=2008
-- expect: none
-- LRM 14.7.2 (distinct static sub-elements have separate sources)
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 3);
begin
  p1: process begin s(0) <= '1'; wait; end process;
  p2: process begin s(1) <= '1'; wait; end process;
  s(2 to 3) <= "00";
end architecture;
