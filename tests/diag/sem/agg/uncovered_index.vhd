-- yavs-test: std=2008
-- expect: E1405 5:38 named aggregate has 3 elements (bounds from its choices), the target 4
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (0 => '1', 1 => '0', 2 => '1');
end package;
