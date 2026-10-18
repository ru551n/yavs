-- yavs-test: std=2008
-- expect: E1623 6:36 dimension 2 does not exist for a 1-dimensional array
-- LRM 16.2
package p is
  signal m : bit_vector(0 to 3);
  constant n : integer := m'length(2);
end package;
