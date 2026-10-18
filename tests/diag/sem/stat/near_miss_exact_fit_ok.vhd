-- yavs-test: std=2008
-- expect: none
-- LRM 4.3.2
package p is
  subtype byte is integer range 0 to 255;
  constant c : byte := 255;
  constant d : bit_vector(0 to 2) := "101";
end package;
