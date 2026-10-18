-- yavs-test: std=2008
-- expect: none
-- LRM 12.4
package p1 is function f (x : integer) return integer; end package;
package p2 is function f (x : boolean) return integer; end package;
use work.p1.all;
use work.p2.all;
package q is
  constant k : integer := f(1);
  constant m : integer := f(true);
end package;
