-- yavs-test: std=2008
-- expect: E1300 7:43 expected `integer`, found `boolean`
-- LRM 6.5.7.2
package g is
  generic (n : integer := 1);
constant c : integer := n; end package;
package i is new work.g generic map (n => true);
