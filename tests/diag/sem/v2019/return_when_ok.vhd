-- yavs-test: std=2019
-- expect: none
-- LRM 2019 10.14: `return when c;` in a procedure
package p is
  procedure clip(x : inout integer);
end package;
package body p is
  procedure clip(x : inout integer) is
  begin
    return when x < 10;
    x := 10;
  end procedure;
end package body;
