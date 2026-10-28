-- yavs-test: std=2008
-- expect: E1621 7:29 attribute `lenght` is not defined for `s`
-- help: an attribute with a similar name exists: `length`
-- LRM 16.2
package p is
  signal s : bit_vector(0 to 3);
  constant c : integer := s'lenght;
end package;
