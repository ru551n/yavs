-- yavs-test: std=2008
-- expect: E1610 6:24 `pr` is not an object
-- LRM 6.6.2
package p is
  procedure pr;
  alias q : integer is pr;
end package;
