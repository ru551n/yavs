-- yavs-test: std=2008
-- expect: E1300 5:23 expected `bit`, found `universal_integer`
-- LRM 9.3.2
package p is
  constant c : bit := 1;
end package;
