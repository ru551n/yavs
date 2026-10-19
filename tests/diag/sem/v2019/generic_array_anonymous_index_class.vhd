-- yavs-test: std=2019
-- expect: E1503 9:44 `color` is not an integer type
-- LRM 2019 6.5.3: an anonymous index type has a class too
package g is
  generic (type vt is array (type is range <>) of type is private);
  subtype s is vt;
end package;
package u is type color is (red, green); type by_color is array (color) of bit; end package;
package i is new work.g generic map (vt => work.u.by_color);
