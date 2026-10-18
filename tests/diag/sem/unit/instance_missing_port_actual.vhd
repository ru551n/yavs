-- yavs-test: std=2008
-- expect: E1311 10:3 port `b` of mode `in` has no actual
-- LRM 6.5.7.3
entity sub is port (a : in bit; b : in bit); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  signal x : bit;
begin
  u : entity work.sub port map (a => x);
end architecture;
