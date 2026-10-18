-- yavs-test: std=2008
-- expect: E1502 9:9 generic type `t` has no actual
-- expect: E1100 9:38 package `g` has no generic `n`
-- LRM 6.5.3
package g is
  generic (type t);
  constant k : integer := 1;
subtype s is t; end package;
package i is new work.g generic map (n => 1);
