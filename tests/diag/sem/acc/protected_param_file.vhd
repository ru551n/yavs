-- yavs-test: std=2008
-- expect: E1223 9:25 a protected method parameter cannot be of file type `ft`
-- LRM 5.6.2 (2008)
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    procedure put (file f : ft);
  end protected;
end package;
