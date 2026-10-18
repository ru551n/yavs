-- yavs-test: std=2008
-- expect: E1002 8:23 architecture `rtl` of entity `sub` not found
-- LRM 11.7
entity sub is end entity;
entity e is end entity;
architecture a of e is
begin
  u : entity work.sub(rtl);
end architecture;
