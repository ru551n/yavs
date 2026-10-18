-- yavs-test: std=2008
-- expect: none
-- LRM 12.3
package p is
  constant n : integer := 3;
  function f (n : integer) return integer;
end package;
package body p is
  function f (n : integer) return integer is
    variable v : integer := n;
  begin
    return v;
  end function;
end package body;
