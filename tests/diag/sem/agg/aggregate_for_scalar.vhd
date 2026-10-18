-- yavs-test: std=2008
-- expect: E1320 5:27 an aggregate cannot have scalar type `integer`
-- LRM 9.3.3
package p is
  constant c : integer := (1, 2);
end package;
