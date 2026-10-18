-- yavs-test: std=2008
-- expect: E1101 12:3 `f` is already declared as a declaration of package `p`
-- LRM 4.8, 12.1
package p is
  function f (x : integer) return integer;
end package;
package body p is
  function f (x : integer) return integer is
  begin
    return x;
  end function;
  function f (x : integer) return integer;
end package body;
