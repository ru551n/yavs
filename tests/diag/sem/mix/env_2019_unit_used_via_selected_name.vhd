-- yavs-test: std=2008 flags=--design-std=2019
-- expect: E1007 5:34 `std.env.tool_name` is not available in VHDL-2008
-- LRM 2019 0.1
package p is
  constant s : string := std.env.tool_name;
end package;
