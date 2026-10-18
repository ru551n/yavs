-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.3
entity e is end entity;
architecture a of e is
  signal s : bit_vector(7 downto 0);
  alias hi : bit_vector(3 downto 0) is s(7 downto 4);
begin
end architecture;
