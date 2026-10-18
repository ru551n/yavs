-- yavs-test: std=2008
-- expect: E1323 5:44 expected `bit`, found `integer`
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 2) := ('0', 1, '1');
end package;
