-- yavs-test: std=2008
-- expect: E1230 5:12 function `twice` has no body
-- LRM 4.2.1, 4.3: the body of a header-instance spec is missing; a body for another name must not satisfy it (#1136)
package p is
  function twice generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t;
end package;
package body p is
  function twmce generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t is
  begin
    return v + v;
  end function;
end package body;
