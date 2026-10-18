-- yavs-test: std=2008
-- expect: none
-- LRM 4.8: deferred constant completed in the body, overload with another profile
package p is
  constant d : integer;
  function f (x : integer) return integer;
end package;
package body p is
  constant d : integer := 1;
  function f (x : integer) return integer is
  begin
    return x;
  end function;
  function f (x : bit) return bit is
  begin
    return x;
  end function;
end package body;
