-- yavs-test: std=2008
-- expect: E1350 6:27 cannot convert `color` to `integer`
-- LRM 9.3.6
package p is
  type color is (red, green);
  constant c : integer := integer(red);
end package;
