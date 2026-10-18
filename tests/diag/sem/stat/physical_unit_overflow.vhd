-- yavs-test: std=2008
-- expect: E1401 9:7 unit scale overflows the base unit
-- LRM 5.2.4.1
package p is
  type big is range 0 to 1000000000
    units
      u;
      k = 1000000000000 u;
      m = 1000000000000 k;
    end units;
end package;
