-- difftest: top=generic_subprog std=2008
package gp is
  function twice generic (type t; function "+"(a, b : t) return t is <>) parameter (x : t) return t;
end package;
package body gp is
  function twice generic (type t; function "+"(a, b : t) return t is <>) parameter (x : t) return t is
  begin
    return x + x;
  end;
end package body;
use work.gp.all;
entity generic_subprog is end;
architecture t of generic_subprog is
  function twice_i is new twice generic map (t => integer);
  function twice_r is new twice generic map (t => real);
begin
  process
  begin
    report integer'image(twice_i(21)) & " " & integer'image(integer(twice_r(1.25) * 4.0));
    wait;
  end process;
end;
