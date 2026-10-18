-- yavs-test: std=2008
-- expect: E1101 7:9 `c` is already declared in this region
-- LRM 6.6
package p is
  constant c : integer := 1;
  constant d : integer := 2;
  alias c is d;
end package;
