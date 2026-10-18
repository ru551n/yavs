-- yavs-test: std=2008
-- expect: E1100 5:27 `x` is not declared
-- LRM 12.3
package p is
  constant c : integer := x;
end package;
