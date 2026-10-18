-- yavs-test: std=2008
-- expect: E1300 8:8 expected `integer`, found `bit`
-- LRM 8.7
entity e is end entity;
architecture a of e is
  signal t : integer;
begin
  t <= <<signal .e.s : bit>>;
end architecture;
