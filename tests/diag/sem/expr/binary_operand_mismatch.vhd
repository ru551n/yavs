-- yavs-test: std=2008
-- expect: E1301 7:27 no operator `+` matches `integer` and `real`
-- LRM 9.2
package p is
  constant a : integer := 1;
  constant b : real := 1.0;
  constant c : integer := a + b;
end package;
