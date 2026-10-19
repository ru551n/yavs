-- yavs-test: std=2008
-- expect: E1641 7:3 template takes 1 constituents
-- LRM 6.10
package p is
  signal s : bit;
  group one is (signal);
  group g : one (s, s);
end package;
