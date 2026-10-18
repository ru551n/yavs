-- yavs-test: std=2008
-- expect: E1401 5:24 negative exponent in integer `**`
-- LRM 9.2.8
package p is
  type t is range 0 to 2 ** (-1);
end package;
