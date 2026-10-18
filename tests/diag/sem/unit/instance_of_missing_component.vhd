-- yavs-test: std=2008
-- expect: E1100 7:7 component `cmp` is not declared
-- LRM 11.7
entity e is end entity;
architecture a of e is
begin
  u : cmp;
end architecture;
