-- yavs-test: std=2019
-- expect: E1503 10:43 designated type of `pb` is `bit`, expected `integer`
-- LRM 2019 6.5.3
package g is
  generic (type t is access integer);
subtype s is t; end package;
package s is
  type pb is access bit;
end package;
package i is new work.g generic map (t => work.s.pb);
