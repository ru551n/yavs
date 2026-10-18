-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1: the expressions of a formal sub-element name are evaluated in the instantiating region
entity c is
  port (d : in bit_vector(7 downto 0));
end entity;
architecture a of c is begin end architecture;
entity e is end entity;
architecture a of e is
  constant w : natural := 4;
  signal s : bit_vector(7 downto 0);
begin
  u : entity work.c port map (d(w - 1 downto 0) => s(w - 1 downto 0), d(7 downto w) => s(7 downto w));
end architecture;
