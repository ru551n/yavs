-- yavs-test: std=2008
-- expect: E1300 5:27 expected `integer`, found `boolean`
-- LRM 6.5.6.2
entity e is
  generic (n : integer := true);
end entity;
