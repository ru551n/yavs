-- rsim-test: std=2008
-- expect: E1621 5:37 attribute `image` is not defined for array type `bit_vector`
-- LRM 16.2
package p is
  constant c : string := bit_vector'image("01");
end package;
