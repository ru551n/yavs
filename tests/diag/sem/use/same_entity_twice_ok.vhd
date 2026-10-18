-- yavs-test: std=2008
-- expect: none
-- LRM 12.4
package p1 is constant c : integer := 1; end package;
use work.p1.all;
use work.p1.c;
package q is
  constant k : integer := c;
end package;
