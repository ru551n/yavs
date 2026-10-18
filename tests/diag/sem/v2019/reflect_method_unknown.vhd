-- yavs-test: std=2019
-- expect: E1100 12:7 protected type `VALUE_MIRROR_PT` has no method `nope`
-- expect: E1100 13:12 protected type `VALUE_MIRROR_PT` has no method `nope`
-- LRM 2019 16.11
use std.reflection.all;
package p is
end package;
package body p is
  procedure t(x : integer) is
    variable m : value_mirror := x'reflect;
  begin
    m.nope;
    m := m.nope;
  end procedure;
end package body;
