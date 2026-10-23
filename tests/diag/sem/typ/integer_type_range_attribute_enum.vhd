-- yavs-test: std=2008
-- expect: E1231 7:19 the range of an integer type definition must be of an integer type
-- LRM 3.1.2 (the bounds of an integer type definition are integers)
package p is
  type color is (red, green, blue);
  type t is array (color) of integer;
  type r is range t'range;
end package;
