-- yavs-test: std=2008
-- expect: E1640 6:13 not a group template
-- LRM 6.10
package p is
  signal s : bit;
  group g : s (s);
end package;
