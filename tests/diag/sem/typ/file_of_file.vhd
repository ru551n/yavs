-- yavs-test: std=2008
-- expect: E1222 6:22 a file cannot contain file type `f1`
-- LRM 5.5
package p is
  type f1 is file of integer;
  type f2 is file of f1;
end package;
