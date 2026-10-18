-- yavs-test: std=2008
-- expect: E1101 9:7 unit `mm` is declared twice
-- LRM 5.2.4.1
package p is
  type len is range 0 to 1000000
    units
      um;
      mm = 1000 um;
      mm = 2000 um;
    end units;
end package;
