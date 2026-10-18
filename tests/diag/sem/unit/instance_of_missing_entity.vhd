-- yavs-test: std=2008
-- expect: E1002 7:19 entity `ghost` not found in library `work`
-- LRM 11.7
entity e is end entity;
architecture a of e is
begin
  u : entity work.ghost;
end architecture;
