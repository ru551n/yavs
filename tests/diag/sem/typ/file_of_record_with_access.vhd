-- yavs-test: std=2008
-- expect: E1222 7:21 a file cannot contain access type `ptr`
-- LRM 5.5
package p is
  type ptr is access integer;
  type r is record a : ptr; end record;
  type f is file of r;
end package;
