-- yavs-test: std=2008
-- expect: E1323 6:22 expected a row aggregate with 2 elements
-- LRM 9.3.3.3
package p is
  type m is array (0 to 1, 0 to 1) of bit;
  constant c : m := ('0', '1');
end package;
