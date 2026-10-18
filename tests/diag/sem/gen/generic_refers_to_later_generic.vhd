-- yavs-test: std=2008
-- expect: E1100 5:27 `b` is not declared at this point
-- LRM 6.5.6
entity e is
  generic (a : integer := b; b : integer := 1);
end entity;
