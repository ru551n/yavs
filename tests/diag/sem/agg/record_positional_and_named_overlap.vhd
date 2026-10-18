-- yavs-test: std=2008
-- expect: E1322 6:25 element `a` is given twice
-- LRM 9.3.3.2
package p is
  type r is record a, b : integer; end record;
  constant c : r := (1, a => 3);
end package;
