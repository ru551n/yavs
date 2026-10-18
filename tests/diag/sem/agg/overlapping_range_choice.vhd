-- yavs-test: std=2008
-- expect: E1327 5:54 choice overlaps an earlier choice
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 to 2 => '1', 2 to 3 => '0');
end package;
