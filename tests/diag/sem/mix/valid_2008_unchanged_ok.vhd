-- yavs-test: std=2008 flags=--design-std=2019
-- expect: none
-- LRM 2019 0.1
use std.env.all;
package p is
  constant t : time := 1 ns;
end package;
