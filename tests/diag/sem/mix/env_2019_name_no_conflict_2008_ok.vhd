-- yavs-test: std=2008 flags=--design-std=2019
-- expect: none
-- LRM 2019 0.1, 12.4
package q is
  constant dir_separator : string := "/";
end package;

use std.env.all;
use work.q.all;
package p is
  constant s : string := dir_separator;
end package;
