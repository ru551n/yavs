-- yavs-test: std=2008
-- expect: E1102 9:27 `c` is made potentially visible by two use clauses
-- LRM 12.4
package p1 is constant c : integer := 1; end package;
package p2 is constant c : integer := 2; end package;
use work.p1.all;
use work.p2.all;
package q is
  constant k : integer := c;
end package;
