-- yavs-test: std=2008
-- expect: E1104 7:12 `t` is not a package, library or package instance
-- LRM 12.4
package p is
  type t is range 0 to 3;
end package;
use work.p.t.all;
package q is
end package;
