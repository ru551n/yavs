-- yavs-test: std=2008
-- expect: E1222 6:22 a file cannot contain multidimensional array type `m`
-- LRM 5.5.1: the element type of a file type is not a multidimensional array (#813)
package p is
  type m is array (1 to 2, 1 to 2) of integer;
  type mf is file of m;
  type v is array (1 to 2) of integer;
  type vf is file of v;
end package;
