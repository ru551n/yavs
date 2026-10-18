-- yavs-test: std=2008
-- expect: E1801 10:5 `wait` is not allowed in a function
-- LRM 10.2
package p is
  function f return integer;
end package;
package body p is
  function f return integer is
  begin
    wait for 1 ns;
    return 1;
  end function;
end package body;
