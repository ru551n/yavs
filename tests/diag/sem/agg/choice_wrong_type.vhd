-- yavs-test: std=2008
-- expect: E1300 5:39 expected index type `natural`, found `boolean`
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := (true => '1', others => '0');
end package;
