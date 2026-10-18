-- yavs-test: std=2008
-- expect: E1321 6:21 element `b` of record `r` has no value
-- LRM 9.3.3.2
package p is
  type r is record a, b : integer; end record;
  constant c : r := (a => 1);
end package;
