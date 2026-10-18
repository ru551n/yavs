-- yavs-test: std=2008
-- expect: E1327 5:59 index 1 is given twice
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 => '1', 1 => '0', 1 => '1', others => '0');
end package;
