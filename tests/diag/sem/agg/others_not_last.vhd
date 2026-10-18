-- yavs-test: std=2008
-- expect: E1325 5:39 `others` must be the last choice
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (others => '0', 1 => '1');
end package;
