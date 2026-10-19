-- yavs-test: std=2019
-- expect: E1223 6:15 a constant parameter cannot be of access type `int_ptr`
-- LRM 6.5.2 (2019 too, user 2026-10-19)
package p is
  type int_ptr is access integer;
  function f (x : int_ptr) return integer;
end package;
