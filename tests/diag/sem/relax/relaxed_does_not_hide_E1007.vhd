-- yavs-test: std=2008 flags=--relaxed,--design-std=2019
-- expect: E1007 4:9 `std.reflection` is not available in VHDL-2008
-- LRM 12.3
use std.reflection.all;
package p is
end package;
