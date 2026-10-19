-- difftest: top=pkg_generics_2019 std=2019
-- VHDL-2019 package generics: generic type classes, array generic types with a
-- generic element.
package class_g is
  generic (
    type enum_t is (<>);
    type elem_t is private;
    type vec_t is array (natural range <>) of elem_t
  );
  function steps return integer;
  function len(v : vec_t) return natural;
end package;
package body class_g is
  function steps return integer is
  begin
    return enum_t'pos(enum_t'high) - enum_t'pos(enum_t'low);
  end function;
  function len(v : vec_t) return natural is
  begin
    return v'length;
  end function;
end package body;

package defs is
  type small is range -3 to 12;
  type color is (red, green, blue, cyan);
  function bool_int(b : boolean) return integer;
end package;
package body defs is
  function bool_int(b : boolean) return integer is
  begin
    if b then return 1; else return 0; end if;
  end function;
end package body;

use work.defs.all;
package cls is new work.class_g
  generic map (enum_t => color, elem_t => bit, vec_t => bit_vector);

use work.defs.all;
entity pkg_generics_2019 is end entity;
architecture t of pkg_generics_2019 is
begin
  process
  begin
    report "steps=" & integer'image(work.cls.steps)
      & " len=" & integer'image(work.cls.len("10101"));
    wait;
  end process;
end architecture;
