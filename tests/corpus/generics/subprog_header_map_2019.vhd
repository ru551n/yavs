-- difftest: top=subprog_header_map_2019 std=2019 ref=hand
-- LRM 4.2.1, 4.4: a subprogram header with a generic list and a generic map declares an
-- instance of the subprogram it describes, called with the instantiated profile (#227).
-- ref=hand: the reference simulators treat such a header as uninstantiated and reject the
-- calls; the expected output is the LRM result, checked by the asserts below.
package shm19_pkg is
  -- Declared here, body in the package body: the body repeats the header.
  function twice generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t;
end package;
package body shm19_pkg is
  function twice generic (type t; function "+" (l, r : t) return t is <>)
    generic map (t => integer) parameter (v : t) return t is
  begin
    return v + v;
  end function;
end package body;
use work.shm19_pkg.all;
entity subprog_header_map_2019 is end;
architecture t of subprog_header_map_2019 is
  -- Two instances under one name overload by their instantiated profiles.
  function code generic (type t is (<>); constant k : integer) generic map (t => boolean, k => 10)
    parameter (v : t) return integer is
  begin
    return k + t'pos(v);
  end function;
  function code generic (type t is (<>); constant k : integer)
    generic map (t => character, k => 1000)
    parameter (v : t) return integer is
  begin
    return k + t'pos(v);
  end function;
  procedure bump generic (constant n : integer) generic map (n => 7)
    parameter (variable x : inout integer) is
  begin
    x := x + n;
  end procedure;
begin
  process
    variable x : integer := twice(21);
  begin
    bump(x);
    assert x = 49 severity failure;
    assert code(true) = 11 and code('A') = 1065 severity failure;
    report "x=" & integer'image(x) & " bool=" & integer'image(code(true))
      & " char=" & integer'image(code('A'));
    wait;
  end process;
end;
