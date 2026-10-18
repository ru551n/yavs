-- yavs-test: std=2008
-- expect: E1101 6:18 `red` is already declared in this region
-- LRM 12.3
package p is
  constant red : integer := 1;
  type color is (red, green);
end package;
