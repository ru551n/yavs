-- yavs-test: std=2008
-- expect: E1214 5:24 type `bit_vector` has 1 index range, the constraint has 2
-- LRM 5.3.2.2
package p is
  signal s : bit_vector(0 to 3, 0 to 1);
end package;
