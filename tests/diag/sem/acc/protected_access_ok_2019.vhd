-- yavs-test: std=2019
-- expect: none
-- LRM 5.6.2 (2008), LCS-2016-004: 2019 accepts access results, access and file parameters (user 2026-10-19)
package p is
  type int_ptr is access integer;
  type ft is file of integer;
  type r is record a : int_ptr; end record;
  type box is protected
    impure function get return int_ptr;
    procedure put (variable x : inout int_ptr);
    procedure put_rec (variable x : inout r);
    procedure put_file (file f : ft);
  end protected;
end package;
