-- yavs-test: std=2008
-- expect: E1101 6:12 `t` is already declared in this region
-- LRM 12.3
package p is
  type t is range 0 to 7;
  constant t : integer := 1;
end package;
