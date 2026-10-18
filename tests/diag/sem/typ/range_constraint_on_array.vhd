-- yavs-test: std=2008
-- expect: E1211 5:25 a range constraint cannot be applied to array type `bit_vector`
-- LRM 6.3
package p is
  signal s : bit_vector range 0 to 3;
end package;
