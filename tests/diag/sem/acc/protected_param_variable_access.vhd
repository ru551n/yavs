-- yavs-test: std=2008
-- expect: E1223 9:29 a protected method parameter cannot be of access type `int_ptr`
-- LRM 5.6.2 (2008)
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    procedure put (variable x : inout int_ptr);
  end protected;
end package;
