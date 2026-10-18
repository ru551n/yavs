-- yavs-test: std=2008
-- expect: E1211 6:15 an index constraint cannot be applied to record type `r`
-- LRM 6.3
package p is
  type r is record a : integer; end record;
  signal s : r(0 to 3);
end package;
