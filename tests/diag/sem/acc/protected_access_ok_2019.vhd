-- yavs-test: std=2019
-- expect: none
-- LRM 5.6.2 (2008)
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    impure function get return int_ptr;
    procedure put (variable x : inout int_ptr);
  end protected;
end package;
