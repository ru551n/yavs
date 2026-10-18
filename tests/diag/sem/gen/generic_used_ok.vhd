-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6
package g is
  generic (n : integer := 1);
  constant w : integer := n;
end package;
package i is new work.g generic map (n => 8);
