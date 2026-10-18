-- yavs-test: std=2008
-- expect: E1102 9:14 `t` is made potentially visible by two use clauses
-- LRM 12.4
package p1 is type t is range 0 to 3; end package;
package p2 is type t is range 0 to 9; end package;
use work.p1.all;
use work.p2.all;
package q is
  signal s : t;
end package;
