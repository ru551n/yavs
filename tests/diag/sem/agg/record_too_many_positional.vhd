-- yavs-test: std=2008
-- expect: E1322 6:28 record `r` has only 2 elements
-- LRM 9.3.3.2
package p is
  type r is record a, b : integer; end record;
  constant c : r := (1, 2, 3);
end package;
