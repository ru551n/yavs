-- yavs-test: std=2008
-- expect: E1802 10:5 a function return needs a value
-- LRM 10.13
package p is
  function f return integer;
end package;
package body p is
  function f return integer is
  begin
    return;
  end function;
end package body;
