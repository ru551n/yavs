-- yavs-test: std=2008
-- expect: E1104 7:12 `c` is not a package, library or package instance
-- LRM 12.4
package p is
  constant c : integer := 1;
end package;
use work.p.c.all;
package q is
end package;
