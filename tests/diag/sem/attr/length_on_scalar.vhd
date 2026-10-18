-- yavs-test: std=2008
-- expect: E1621 6:29 attribute `length` is not defined for scalar `a`
-- LRM 16.2
package p is
  constant a : integer := 1;
  constant c : integer := a'length;
end package;
