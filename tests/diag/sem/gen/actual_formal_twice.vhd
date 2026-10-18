-- yavs-test: std=2008
-- expect: E1310 7:46 generic `n` is associated twice
-- LRM 6.5.7.2
package g is
  generic (n : integer := 1);
constant c : integer := n; end package;
package i is new work.g generic map (n => 1, n => 2);
