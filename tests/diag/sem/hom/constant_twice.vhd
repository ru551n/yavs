-- yavs-test: std=2008
-- expect: E1101 6:12 `c` is already declared in this region
-- LRM 12.3
package p is
  constant c : integer := 1;
  constant c : integer := 2;
end package;
