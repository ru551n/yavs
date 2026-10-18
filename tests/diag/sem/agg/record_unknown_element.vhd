-- yavs-test: std=2008
-- expect: E1100 6:30 record `r` has no element `zz`
-- LRM 9.3.3.2
package p is
  type r is record a, b : integer; end record;
  constant c : r := (a => 1, zz => 2);
end package;
