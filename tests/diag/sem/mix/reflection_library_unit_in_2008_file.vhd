-- yavs-test: std=2008 flags=--design-std=2019
-- expect: E1007 4:9 `std.reflection` is not available in VHDL-2008
-- LRM 2019 0.1
use std.reflection.all;
package p is
end package;
