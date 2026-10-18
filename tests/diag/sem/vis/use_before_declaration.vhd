-- yavs-test: std=2008
-- expect: E1100 5:27 `b` is not declared at this point
-- LRM 12.2
package p is
  constant a : integer := b;
  constant b : integer := 1;
end package;
