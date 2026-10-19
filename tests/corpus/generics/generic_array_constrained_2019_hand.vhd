-- difftest: top=generic_array_constrained_2019_hand std=2019 ref=hand
-- ref=hand: no reference simulator accepts a generic array type with an index constraint
-- (`array (0 to 3) of ...`, LRM 2019 6.5.3) or an anonymous designated type; the
-- expected output is the LRM result.
package q is
  generic (type ct is array (0 to 3) of type is (<>);
           type pt is access type is private);
  function second (c : ct) return ct'element;
  function deref (p : pt) return pt'designated_subtype;
end package;
package body q is
  function second (c : ct) return ct'element is
    variable v : ct := c;
  begin
    v(0) := v(1);
    return v(0);
  end function;
  function deref (p : pt) return pt'designated_subtype is begin return p.all; end function;
end package body;
package u is
  type quad is array (0 to 3) of character;
  type int_ptr is access integer;
end package;
use work.u.all;
package qi is new work.q generic map (ct => quad, pt => int_ptr);
use work.u.all;
entity generic_array_constrained_2019_hand is end;
architecture t of generic_array_constrained_2019_hand is
begin
  process
    variable p : int_ptr := new integer'(42);
  begin
    report character'image(work.qi.second("abcd")) & " " & integer'image(quad'length);
    report integer'image(work.qi.deref(p));
    wait;
  end process;
end architecture;
