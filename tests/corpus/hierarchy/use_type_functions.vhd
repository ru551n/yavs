-- difftest: top=tb_use_type_functions std=2008
-- A use clause naming a type makes the type visible, not the other subprograms declared next to
-- it: with only the type named, `to_slv(make(n))` below has one interpretation, the conversion
-- of the standard logic package applied to the overload by return type being defined.
library ieee;
use ieee.std_logic_1164.all;

package pk_utf is
  type word is array (natural range <>) of std_ulogic;
  function to_slv(x : word) return std_logic_vector;
end package;
package body pk_utf is
  function to_slv(x : word) return std_logic_vector is
    variable r : std_logic_vector(x'range);
  begin
    for i in x'range loop
      r(i) := x(i);
    end loop;
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pk_utf.word;

package pk_utf2 is
  function make(n : natural) return word;
  function make(n : natural) return std_logic_vector;
end package;
package body pk_utf2 is
  function make(n : natural) return word is
    variable r : word(1 to 4) := (others => '0');
  begin
    r(1) := '1';
    return r;
  end function;
  function make(n : natural) return std_logic_vector is
  begin
    -- Never taken (it would recurse); the point is that it analyses.
    if n = 12345 then
      return to_slv(make(n));
    end if;
    return "1000";
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pk_utf2.all;

entity tb_use_type_functions is end;
architecture t of tb_use_type_functions is
begin
  process
    variable v : std_logic_vector(1 to 4);
  begin
    v := make(0);
    assert v = "1000" report "wrong value" severity failure;
    report "ok";
    wait;
  end process;
end;
