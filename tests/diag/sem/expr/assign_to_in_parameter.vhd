-- yavs-test: std=2008
-- expect: E1360 10:5 cannot assign to parameter `x` of mode `in`
-- LRM 10.6
package p is
  procedure pr (x : in integer);
end package;
package body p is
  procedure pr (x : in integer) is
  begin
    x := 1;
  end procedure;
end package body;
