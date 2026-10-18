-- yavs-test: std=2008
-- expect: E1812 10:3 unresolved signal `s` has multiple sources
-- LRM 14.7.2
entity e is end entity;
architecture a of e is
  type r is record a, b : bit; end record;
  signal s : r;
begin
  s.a <= '1';
  s.a <= '0';
  s.b <= '0';
end architecture;
