-- yavs-test: std=2008
-- expect: none
-- LRM 12.4
-- An explicitly declared operator next to the type is visible through a use clause naming the
-- type (the reference simulators call it, catalogue use-type-operations).
package p is
  type t is (a, b);
  function "=" (x, y : t) return boolean;
end package;
package body p is
  function "=" (x, y : t) return boolean is
  begin
    return true;
  end function;
end package body;
use work.p.t;
package q is
  function same(x, y : t) return boolean;
end package;
package body q is
  function same(x, y : t) return boolean is begin return x = y; end function;
end package body;
