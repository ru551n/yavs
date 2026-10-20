-- difftest: top=tb_use_type_operations std=2008
-- A use clause naming a subtype also makes the operations of its base type visible,
-- including a user-defined "=" declared next to the type.
package pk_uto is
  type t_abc is (a, b, c);
  subtype t_sub is t_abc range a to c;
  function "=" (x, y : t_abc) return boolean;
end package;
package body pk_uto is
  function "=" (x, y : t_abc) return boolean is
  begin
    report "called overloaded =";
    return true;
  end function;
end package body;

use work.pk_uto.t_sub;
entity tb_use_type_operations is end;
architecture t of tb_use_type_operations is
begin
  process
    variable x : t_sub := a;
  begin
    wait for 1 ns;
    assert x = c;
    wait;
  end process;
end;
