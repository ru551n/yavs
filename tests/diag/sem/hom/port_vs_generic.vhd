-- yavs-test: std=2008
-- expect: E1101 6:9 `n` is already declared as a generic of `e`
-- LRM 12.3
entity e is
  generic (n : integer := 1);
  port (n : in bit);
end entity;
