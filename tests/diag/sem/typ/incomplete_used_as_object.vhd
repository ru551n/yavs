-- yavs-test: std=2008
-- expect: E1220 6:14 incomplete type `t` can only be used as an access designated type
-- LRM 5.4.2
package p is
  type t;
  signal s : t;
  type t is range 0 to 3;
end package;
