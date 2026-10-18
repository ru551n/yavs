-- yavs-test: std=2008
-- expect: E1328 5:38 index 3 is not covered and there is no `others`
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 => '1', 1 => '0', 2 => '1');
end package;
