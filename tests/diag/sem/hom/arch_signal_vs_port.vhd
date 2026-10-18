-- yavs-test: std=2008
-- expect: E1101 8:10 `a` is already declared as a port of `e`
-- LRM 12.3
entity e is
  port (a : in bit);
end entity;
architecture r of e is
  signal a : bit;
begin
end architecture;
