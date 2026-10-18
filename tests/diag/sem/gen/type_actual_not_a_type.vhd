-- yavs-test: std=2008
-- expect: E1201 7:43 `true` is not a type
-- LRM 6.5.3
package g is
  generic (type t);
subtype s is t; end package;
package i is new work.g generic map (t => true);
