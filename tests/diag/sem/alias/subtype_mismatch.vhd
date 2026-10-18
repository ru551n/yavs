-- yavs-test: std=2008
-- expect: E1300 6:13 alias subtype `integer` does not match `bit_vector`
-- LRM 6.6.2
package p is
  constant a : bit_vector(0 to 3) := "0000";
  alias b : integer is a;
end package;
