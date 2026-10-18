-- yavs-test: std=2008
-- expect: E1100 7:38 package `g` has no generic `zz`
-- LRM 6.5.7.2
package g is
  generic (n : integer := 1);
constant c : integer := n; end package;
package i is new work.g generic map (zz => 1);
