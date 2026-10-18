-- yavs-test: std=2008
-- expect: E1300 7:18 expected an index of type `color`, found `integer`
-- LRM 5.3.2.2
package p is
  type color is (red, green, blue);
  type arr is array (color range <>) of bit;
  signal s : arr(0 to 1);
end package;
