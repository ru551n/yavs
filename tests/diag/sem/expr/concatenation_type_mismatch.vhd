-- yavs-test: std=2008
-- expect: E1301 5:38 no operator `&` matches `string` and `integer`
-- LRM 9.2.5
package p is
  constant c : bit_vector(0 to 1) := "1" & 1;
end package;
