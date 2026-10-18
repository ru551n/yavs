-- yavs-test: std=2008
-- expect: none
-- LRM 12.2
package p is
  constant a : integer := 1;
  constant b : integer := a + 1;
end package;
