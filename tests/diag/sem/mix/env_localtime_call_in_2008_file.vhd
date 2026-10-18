-- yavs-test: std=2008 flags=--design-std=2019
-- expect: E1007 12:10 `std.env.time_to_seconds` is not available in VHDL-2008
-- LRM 2019 0.1
use std.env.all;
package p is
  procedure q;
end package;
package body p is
  procedure q is
    variable t : time;
  begin
    t := time_to_seconds(1 ns) * 1 ns;
  end procedure;
end package body;
