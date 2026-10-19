-- yavs-test: std=2008
-- expect: E1102 9:27 `red` is made potentially visible by two use clauses
-- LRM 12.4
package p1 is constant red : integer := 1; end package;
package p2 is type color is (red, green); end package;
use work.p1.all;
use work.p2.all;
package q is
  constant k : integer := red;
end package;
