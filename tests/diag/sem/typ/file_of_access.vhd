-- yavs-test: std=2008
-- expect: E1222 6:21 a file cannot contain access type `ptr`
-- LRM 5.5
package p is
  type ptr is access integer;
  type f is file of ptr;
end package;
