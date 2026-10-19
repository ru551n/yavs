-- yavs-test: std=2019
-- expect: none
-- LRM 4.2.1
package p is
  type int_ptr is access integer;
  type rec is record a : int_ptr; end record;
  impure function f return int_ptr;
  impure function g return rec;
end package;
