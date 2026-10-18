-- yavs-test: std=2008
-- expect: E1326 5:44 positional and named associations cannot be mixed
-- LRM 9.3.3.3
package p is
  constant c : bit_vector(0 to 3) := ('1', 1 => '0', others => '0');
end package;
