-- yavs-test: std=2008
-- expect: E1214 6:25 array `c` has 1 dimension, found 2 indexes
-- LRM 8.4
package p is
  constant c : bit_vector(0 to 3) := "0000";
  constant d : bit := c(0, 1);
end package;
