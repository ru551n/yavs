-- yavs-test: std=2019
-- expect: E1503 10:43 `r` is not a discrete type
-- LRM 2019 6.5.3
package g is
  generic (type t is (<>));
subtype s is t; end package;
package s is
  type r is record a : bit; end record;
end package;
package i is new work.g generic map (t => work.s.r);
