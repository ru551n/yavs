-- yavs-test: std=2008
-- expect: none
-- LRM 12.4
package g is
  generic (n : integer := 4);
  constant w : integer := n;
end package;
package i is new work.g generic map (n => 8);
use work.i.all;
package q is
  constant k : integer := w;
end package;
