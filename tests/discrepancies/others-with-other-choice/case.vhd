-- yavs-test: std=2008
-- expect: E1325 5:43 `others` must be alone in its association
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 | others => '1');
end package;
