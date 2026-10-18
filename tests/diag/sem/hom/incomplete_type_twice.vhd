-- yavs-test: std=2008
-- expect: E1101 7:8 `t` is already declared in this region
-- LRM 5.4.2
package p is
  type t;
  type t is range 0 to 3;
  type t is range 0 to 4;
end package;
