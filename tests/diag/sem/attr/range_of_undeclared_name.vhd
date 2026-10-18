-- yavs-test: std=2008
-- expect: E1100 5:27 `gx` is not declared
-- LRM 16.2.3 (an undeclared prefix of 'range is E1100, not E1361)
entity e is
  port (i : in bit_vector(gx'range));
end entity;
