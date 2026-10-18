-- yavs-test: std=2008
-- expect: E1403 6:24 value 256 is outside subtype `byte` range 0 to 255
-- LRM 4.3.2
package p is
  subtype byte is integer range 0 to 255;
  constant c : byte := 256;
end package;
