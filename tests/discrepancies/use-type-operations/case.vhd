-- A use clause naming a subtype: which operations of the type does it make visible?
package pk is
  type t_abc is (a, b, c);
  subtype t_sub is t_abc range a to c;
  function "=" (x, y : t_abc) return boolean;
end package;
package body pk is
  function "=" (x, y : t_abc) return boolean is
  begin
    return true;
  end function;
end package body;

use work.pk.t_sub;
entity top is end entity;
architecture t of top is
begin
  process
    variable x : t_sub := a;
  begin
    report "x = c is " & boolean'image(x = c);
    wait;
  end process;
end architecture;
