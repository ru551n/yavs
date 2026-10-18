-- yavs-test: std=2008
-- expect: E1502 9:9 generic `n` has no actual and no default
-- expect: E1100 9:38 package `g` has no generic `m`
-- LRM 6.5.7.2
package g is
  generic (n : integer);
  constant w : integer := n;
end package;
package i is new work.g generic map (m => 1);
