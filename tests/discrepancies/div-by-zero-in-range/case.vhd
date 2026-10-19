-- yavs-test: std=2008
-- expect: E1401 5:24 division by zero in constant expression
-- LRM 9.2.7
package p is
  type t is range 0 to 4 / 0;
end package;
