-- yavs-test: std=2008
-- expect: E1904 5:19 shared variable `sv` must be of a protected type
-- LRM 6.4.2.4
package p is
  shared variable sv : integer;
end package;
