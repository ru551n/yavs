-- yavs-test: std=2008
-- expect: E1332 6:21 `c` is not a physical unit
-- LRM 5.2.4
package p is
  constant c : integer := 1;
  type t is range 0 c to 9;
end package;
