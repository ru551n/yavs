-- yavs-test: std=2008
-- expect: E1330 5:31 character `a` is not a literal of type `bit`
-- LRM 15.7
package p is
  constant c : bit_vector := "a01";
end package;
