-- yavs-test: std=2008
-- expect: E1002 7:7 architecture `nope` of entity `e` not found
-- LRM 3.4
entity e is end entity;
architecture a of e is begin end architecture;
configuration cfg of e is
  for nope
  end for;
end configuration;
