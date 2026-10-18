-- yavs-test: std=2008
-- expect: E1323 6:22 `others` covers elements of different types
-- LRM 9.3.3.2
package p is
  type r is record a : integer; b : bit; end record;
  constant c : r := (others => 0);
end package;
