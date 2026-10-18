-- yavs-test: std=2008
-- expect: E1300 6:30 expected `integer`, found `color`
-- LRM 5.2.1
package p is
  type color is (red, green);
  subtype s is integer range red to green;
end package;
