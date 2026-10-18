-- yavs-test: std=2008
-- expect: E1100 8:24 type `nothing` is not declared
-- LRM 8.7
entity e is end entity;
architecture a of e is
  signal t : bit;
begin
  t <= <<signal .e.s : nothing>>;
end architecture;
