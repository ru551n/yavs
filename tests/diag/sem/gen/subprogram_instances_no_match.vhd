-- yavs-test: std=2008
-- expect: E1301 18:27 no instance takes a real
-- LRM 12.5: only the instantiated profiles take part in overload resolution (#28)
package p is
  function f generic (type t) parameter (v : t) return integer;
end package;
package body p is
  function f generic (type t) parameter (v : t) return integer is
  begin
    return 0;
  end;
end package body;
use work.p.all;
package q is
  type rec is record a : integer; end record;
  function f is new f generic map (t => integer);
  function f is new f generic map (t => rec);
  constant c : integer := f(1.5);
end package;
