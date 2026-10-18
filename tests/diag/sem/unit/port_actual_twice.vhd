-- yavs-test: std=2008
-- expect: E1310 10:41 formal `a` is associated twice
-- LRM 6.5.7.3
entity sub is port (a : in bit); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  signal x, y : bit;
begin
  u : entity work.sub port map (a => x, a => y);
end architecture;
