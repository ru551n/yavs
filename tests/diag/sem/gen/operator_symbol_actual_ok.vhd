-- rsim-test: std=2008
-- expect: none
-- LRM 6.5.7.2 (OSVVM ScoreboardPkg_int: Match => "="; NVC and GHDL accept)
package g is
  generic (type t; function match (l, r : t) return boolean);
  function same (a, b : t) return boolean;
end package;
package body g is
  function same (a, b : t) return boolean is
  begin
    return match(a, b);
  end function;
end package body;
package i is new work.g generic map (t => integer, match => "=");
