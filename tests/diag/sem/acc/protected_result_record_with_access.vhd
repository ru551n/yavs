-- yavs-test: std=2008
-- expect: E1223 9:32 a protected method result cannot be of type `r`, which contains access type `int_ptr`
-- LRM 5.6.2 (2008)
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    impure function get return r;
  end protected;
end package;
