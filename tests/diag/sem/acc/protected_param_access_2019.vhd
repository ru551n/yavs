-- yavs-test: std=2019
-- expect: E1223 7:29 a protected method parameter cannot be of access type `int_ptr`
-- LRM 5.6.2 (2019 too, user 2026-10-19)
package p is
  type int_ptr is access integer;
  type box is protected
    procedure put (variable x : inout int_ptr);
  end protected;
end package;
