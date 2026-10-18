-- yavs-test: std=2008
-- expect: E1350 5:38 cannot convert `integer` to `bit_vector`
-- LRM 9.3.6
package p is
  constant c : bit_vector(0 to 3) := bit_vector(5);
end package;
