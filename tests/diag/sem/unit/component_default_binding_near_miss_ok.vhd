-- yavs-test: std=2008
-- expect: none
-- LRM 7.3.3
entity sub is port (a : in bit); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component sub is port (a : in bit); end component;
  signal x : bit;
begin
  u : sub port map (a => x);
end architecture;
