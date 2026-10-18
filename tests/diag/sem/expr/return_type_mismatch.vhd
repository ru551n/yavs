-- yavs-test: std=2008
-- expect: E1300 10:12 expected `integer`, found `boolean`
-- LRM 10.13
package p is
  function f return integer;
end package;
package body p is
  function f return integer is
  begin
    return true;
  end function;
end package body;
