-- yavs-test: std=2008
-- expect: none
-- LRM 4.2.1: near miss, the body is present for the header-instance spec in a package (#1136)
package p is
  function twice generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t;
end package;
package body p is
  function twice generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t is
  begin
    return v + v;
  end function;
end package body;
