-- yavs-test: std=2008
-- expect: none
-- LRM 9.2.8
package p is
  constant a : integer := integer'high - 1;
  constant b : integer := a + 1;
end package;
