-- yavs-test: std=2008
-- expect: E1100 5:14 `nothing` is not declared
-- LRM 6.6
package p is
  alias b is nothing;
end package;
