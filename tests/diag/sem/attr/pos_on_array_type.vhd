-- yavs-test: std=2008
-- expect: E1621 5:38 attribute `pos` is not defined for array type `bit_vector`
-- LRM 16.2
package p is
  constant c : integer := bit_vector'pos("0");
end package;
