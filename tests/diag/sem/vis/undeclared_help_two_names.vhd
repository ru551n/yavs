-- yavs-test: std=2008
-- expect: E1100 8:31 `datb` is not declared
-- help: similar names exist: `data` (constant), `datc` (constant)
-- LRM 12.3
package p is
  constant data : integer := 1;
  constant datc : integer := 2;
  constant other : integer := datb;
end package;
