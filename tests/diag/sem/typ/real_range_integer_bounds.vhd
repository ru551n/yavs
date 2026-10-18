-- yavs-test: std=2008
-- expect: E1300 5:27 expected `real`, found `universal_integer`
-- LRM 5.2.5
package p is
  subtype s is real range 1 to 5.0;
end package;
