-- yavs-test: std=2008
-- expect: none
-- LRM 8.7
entity sub is end entity;
architecture r of sub is
  signal s : bit;
begin
end architecture;
entity e is end entity;
architecture a of e is
  signal t : bit;
begin
  u : entity work.sub;
  t <= <<signal .e.u.s : bit>>;
end architecture;
