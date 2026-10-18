-- yavs-test: std=2008
-- expect: E1350 7:30 cannot convert `bit_vector` to `iv`
-- LRM 9.3.6
package p is
  type iv is array (natural range <>) of integer;
  constant b : bit_vector(0 to 1) := "10";
  constant c : iv(0 to 1) := iv(b);
end package;
