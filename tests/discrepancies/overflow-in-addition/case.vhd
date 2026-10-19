-- yavs-test: std=2008
-- expect: W1401 5:27 integer overflow in constant expression, checked at run time
-- LRM 9.2.8
package p is
  constant c : integer := integer'high + 1;
end package;
