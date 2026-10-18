-- yavs-test: std=2008
-- expect: E1700 8:7 `s` is not a component, entity or configuration
-- LRM 11.7
entity e is end entity;
architecture a of e is
  signal s : integer;
begin
  u : s;
end architecture;
