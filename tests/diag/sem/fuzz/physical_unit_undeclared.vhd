-- yavs-test: std=2008
-- expect: E1100 5:21 `xx` is not declared
-- LRM 5.2.4
package p is
  type t is range 0 xx to 9;
end package;
