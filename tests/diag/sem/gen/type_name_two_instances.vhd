-- yavs-test: std=2008
-- expect: E1300 13:27 expected `work.i2.rec`, found `work.i1.rec`
-- LRM 9.3.6
package g is
  generic (n : natural);
  type rec is record a : integer; end record;
constant c : natural := n; end package;
package i1 is new work.g generic map (n => 1);
package i2 is new work.g generic map (n => 2);
use work.i1, work.i2;
package q is
  constant c1 : i1.rec := (a => 1);
  constant c2 : i2.rec := c1;
end package;
