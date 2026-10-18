-- yavs-test: std=2008
-- expect: E1300 6:29 `c` has type `integer` and cannot be indexed
-- LRM 8.4
package p is
  constant c : integer := 1;
  constant d : integer := c(0);
end package;
