-- yavs-test: std=2008
-- expect: E1101 8:12 `f` with the same profile is already declared
-- LRM 4.5.3
package p is
end package;
package body p is
  function f return integer is begin return 1; end function;
  function f return integer is begin return 2; end function;
end package body;
