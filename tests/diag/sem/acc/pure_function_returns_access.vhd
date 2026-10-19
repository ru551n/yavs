-- yavs-test: std=2008
-- expect: E1830 7:26 the result of pure function `f` is or contains access type `int_ptr`
-- expect: E1830 8:26 the result of pure function `g` is or contains access type `int_ptr`
-- LRM 4.2.1
package p is
  type int_ptr is access integer;
  pure function f return int_ptr;
  pure function g return int_ptr;
end package;
