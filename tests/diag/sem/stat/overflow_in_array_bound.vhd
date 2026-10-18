-- yavs-test: std=2008
-- expect: E1401 5:25 integer overflow in constant expression
-- LRM 9.2.7
package p is
  type t is array (0 to integer'high * 2) of bit;
end package;
