-- yavs-test: std=2019
-- expect: E1503 7:43 `real` is not a discrete type
-- LRM 6.5.3
package g is
  generic (type t is (<>));
subtype s is t; end package;
package i is new work.g generic map (t => real);
