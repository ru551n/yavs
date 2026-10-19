-- yavs-test: std=2008
-- expect: E1100 6:18 nothing is not declared
-- LRM 6.10
package p is
  group one is (signal);
  group g : one (nothing);
end package;
