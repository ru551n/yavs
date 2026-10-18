-- yavs-test: std=2008
-- expect: E1403 7:26 value 20 is outside subtype `nibble` range 0 to 15
-- LRM 4.3.2
package p is
  subtype nibble is integer range 0 to 15;
  constant a : integer := 10;
  constant c : nibble := a * 2;
end package;
