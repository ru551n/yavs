-- yavs-test: std=2008 flags=--design-std=2019
-- expect: E1007 5:13 `std.env.tool_name` is not available in VHDL-2008
-- help: the design is elaborated with the VHDL-2019 std because another unit is VHDL-2019
-- LRM 2019 0.1
use std.env.tool_name;
package p is
end package;
