-- yavs-test: std=2008
-- expect: E1300 6:24 expected a unit of `time`, found unit `m` of `len`
-- LRM 5.2.4
package p is
  type len is range 0 to 100 units m; end units;
  constant c : time := 5 m;
end package;
