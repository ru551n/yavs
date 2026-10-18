-- yavs-test: std=2019
-- expect: E1621 9:43 `'reflect` of a type mark needs a fully constrained subtype
-- LRM 2019 16.2
use std.reflection.all;
package p is
end package;
package body p is
  procedure t is
    variable s : subtype_mirror := string'reflect;
  begin
  end procedure;
end package body;
