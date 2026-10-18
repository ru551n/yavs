-- yavs-test: std=2008
-- expect: E1325 6:38 `others` covers no element
-- LRM 9.3.3.2
package p is
  type r is record a, b : integer; end record;
  constant c : r := (a => 1, b => 2, others => 3);
end package;
