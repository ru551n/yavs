-- yavs-test: std=2019
-- expect: E1503 7:43 `integer` is not a floating-point type
-- LRM 2019 6.5.3
package g is
  generic (type t is range <> . <>);
subtype s is t; end package;
package i is new work.g generic map (t => integer);
