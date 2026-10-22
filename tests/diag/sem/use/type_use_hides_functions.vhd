-- yavs-test: std=2008
-- expect: E1100 22:52 `to_i` is not declared
-- LRM 12.4
-- A use clause naming a type makes the type, its literals and its implicit operations visible,
-- not the other subprograms declared next to it (all four reference simulators agree).
package p is
  type t is (a, b);
  function to_i(x : t) return integer;
end package;
package body p is
  function to_i(x : t) return integer is
  begin
    return t'pos(x);
  end function;
end package body;
use work.p.t;
package q is
  constant k : integer := 3;
  function f(x : t) return integer;
end package;
package body q is
  function f(x : t) return integer is begin return to_i(x); end function;
end package body;
