-- yavs-test: std=2019
-- expect: none
-- LRM 5.6.2
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    procedure put (constant x : in int_ptr);
  end protected;
end package;
