-- yavs-test: std=2008
-- expect: E1100 5:27 `a` is not declared at this point
-- LRM 12.2
package p is
  constant a : integer := a + 1;
end package;
