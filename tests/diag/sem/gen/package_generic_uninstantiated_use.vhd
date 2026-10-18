-- yavs-test: std=2008
-- expect: E1505 9:32 `g` is a generic package and must be instantiated before use
-- LRM 4.9
package g is
  generic (n : integer := 1);
  constant w : integer := n;
end package;
package q is
  constant k : integer := work.g.w;
end package;
