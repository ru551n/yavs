-- yavs-test: std=2008
-- expect: E1223 7:12 a constant cannot be of type `r`, which contains access type `int_ptr`
-- LRM 6.4.2.2
package p is
  type int_ptr is access integer;
  type r is record a : int_ptr; end record;
  constant c : r := (a => null);
end package;
