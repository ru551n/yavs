-- yavs-test: std=2008 flags=--relaxed
-- expect: W1830 6:26 the result of pure function `f` is or contains access type `int_ptr`
-- LRM 4.2.1
package p is
  type int_ptr is access integer;
  pure function f return int_ptr;
end package;
