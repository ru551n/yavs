-- yavs-test: std=2008
-- expect: E1100 5:27 function `compute` is not declared
-- LRM 12.3
package p is
  constant c : integer := compute(1);
end package;
