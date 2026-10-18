-- yavs-test: std=2008
-- expect: none
-- LRM 4.2.1: a subprogram header with a generic list and a generic map declares an
-- instance of the subprogram it describes; a separate body repeats the header (#227)
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
use work.p.all;
entity e is end;
architecture a of e is
  -- Two header instances overload by their instantiated profiles.
  function first generic (type t) generic map (t => bit) parameter (v : t) return integer is
  begin
    return 1;
  end function;
  function first generic (type t) generic map (t => boolean) parameter (v : t) return integer is
  begin
    return 2;
  end function;
  procedure add generic (constant n : integer) generic map (n => 7)
    parameter (variable x : inout integer) is
  begin
    x := x + n;
  end procedure;
begin
  process
    variable x : integer := twice(21);
  begin
    add(x);
    assert x = 49 and first('1') = 1 and first(true) = 2;
    wait;
  end process;
end;
