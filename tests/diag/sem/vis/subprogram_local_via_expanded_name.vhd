-- yavs-test: std=2008
-- expect: E1103 13:29 `v` exists but is not visible here
-- LRM 8.3
package p is
  function f return integer;
end package;
package body p is
  function f return integer is
    variable v : integer := 1;
  begin
    return v;
  end function;
  constant k : integer := f.v;
end package body;
