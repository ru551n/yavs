-- yavs-test: std=2008
-- expect: E1403 7:24 value `blue` is outside subtype `warm`
-- LRM 4.3.2
package p is
  type color is (red, green, blue);
  subtype warm is color range red to green;
  constant c : warm := blue;
end package;
