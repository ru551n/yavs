-- rsim-test: std=2008
-- expect: E1302 10:27 call of `f` is ambiguous
-- help: qualify the call with the package: `work.p1.f(1)`
-- LRM 12.4
package p1 is function f (x : integer) return integer; end package;
package p2 is function f (x : integer) return integer; end package;
use work.p1.all;
use work.p2.all;
package q is
  constant k : integer := f(1);
end package;
