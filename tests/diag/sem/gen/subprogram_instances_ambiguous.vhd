-- yavs-test: std=2008
-- expect: E1302 18:27 f(1) matches both the integer and the small instance
-- LRM 12.5: instances of one generic function overload, so a call both accept is ambiguous (#28)
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
  type small is range 0 to 9;
  function f is new f generic map (t => integer);
  function f is new f generic map (t => small);
  constant c : integer := f(1);
end package;
