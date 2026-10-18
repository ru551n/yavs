-- yavs-test: std=2008
-- expect: E1100 7:12 `nothere` is not declared in package `p`
-- LRM 12.4
package p is
  constant c : integer := 1;
end package;
use work.p.nothere;
package q is
end package;
