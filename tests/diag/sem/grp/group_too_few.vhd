-- yavs-test: std=2008
-- expect: E1641 7:3 template takes 2 constituents
-- LRM 6.10
package p is
  signal s : bit;
  group pair is (signal, signal);
  group g : pair (s);
end package;
