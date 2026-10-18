-- yavs-test: std=2008
-- expect: E1100 10:33 entity `sub` has no port `b`
-- LRM 6.5.7
entity sub is port (a : in bit); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  signal x : bit;
begin
  u : entity work.sub port map (b => x);
end architecture;
