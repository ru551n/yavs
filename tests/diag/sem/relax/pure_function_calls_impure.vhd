-- yavs-test: std=2008
-- expect: E1830 12:12 pure function `f` calls impure function `g`
-- LRM 4.3
package p is
  impure function g return integer;
  function f return integer;
end package;
package body p is
  impure function g return integer is begin return 1; end function;
  function f return integer is
  begin
    return g;
  end function;
end package body;
