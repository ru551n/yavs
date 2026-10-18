-- yavs-test: std=2008
-- expect: E1300 5:24 expected `real`, found `universal_integer`
-- LRM 9.3.2
package p is
  constant c : real := 1;
end package;
