-- yavs-test: std=2008
-- expect: E1321 6:21 element `c` of record `r` has no value
-- LRM 9.3.3.2
package p is
  type r is record a, b, c : integer; end record;
  constant k : r := (1, 2);
end package;
