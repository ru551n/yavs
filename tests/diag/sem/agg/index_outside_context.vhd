-- rsim-test: std=2008
-- expect: E1403 5:79 index 4 is outside range 0 to 3
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 => '1', 1 => '0', 2 => '1', 3 => '0', 4 => '1');
end package;
