-- yavs-test: std=2008
-- expect: E1100 5:27 `x` is not declared
-- LRM 12.3 (one error per design unit and name)
package p is
  constant a : integer := x;
  constant b : integer := x + 1;
  constant c : integer := x * 2;
end package;
