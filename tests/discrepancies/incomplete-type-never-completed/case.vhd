-- yavs-test: std=2008
-- expect: E1220 5:8 incomplete type `t` is never completed
-- LRM 5.4.2
package p is
  type t;
end package;
