-- difftest: top=physical std=2019
-- std.reflection physical mirrors (LRM 2019 16.11), TIME included. TIME mirror values are in
-- units of the resolution limit (docs/decisions.md "Reflection: TIME values in mirrors"); with
-- the default femtosecond resolution this is the femtosecond count.
use std.reflection.all;
entity physical is end entity;

architecture test of physical is
  type dist is range -100 to 1000000
    units
      um;
      mm = 1000 um;
      cm = 10 mm;
    end units;
  subtype short is dist range 0 um to 5 cm;
begin
  process
    variable t : time := 5 ns;
    variable d : short := 2 cm;
    variable pm : physical_subtype_mirror;
    variable pv : physical_value_mirror;
  begin
    pv := t'reflect.to_physical;
    report "t value " & integer'image(pv.value) & " unit " & index'image(pv.unit_index)
      & " image " & pv.image & " sub " & pv.get_subtype_mirror.simple_name;
    pm := time'reflect.to_physical;
    report "time units " & index'image(pm.units_length);
    for i in 1 to pm.units_length loop
      report "unit " & index'image(i) & " " & pm.unit_name(i) & " scale "
        & integer'image(pm.scale(i));
    end loop;
    report "ns idx " & index'image(pm.unit_index("ns")) & " scale us "
      & integer'image(pm.scale("US"));
    pm := delay_length'reflect.to_physical;
    report "delay_length " & pm.simple_name & " low " & integer'image(pm.low.value)
      & " asc " & boolean'image(pm.ascending);
    pv := d'reflect.to_physical;
    report "d value " & integer'image(pv.value) & " image " & pv.image & " sub "
      & pv.get_subtype_mirror.simple_name;
    pm := short'reflect.to_physical;
    report "short left " & integer'image(pm.left.value) & " right " & integer'image(pm.right.value)
      & " high " & pm.high.image & " len " & index'image(pm.length);
    pm := dist'reflect.to_physical;
    report "dist low " & integer'image(pm.low.value) & " scale cm " & integer'image(pm.scale("cm"))
      & " name " & pm.unit_name(3);
    assert pv.to_value_mirror.get_value_class = class_physical severity failure;
    assert pm.to_subtype_mirror.get_type_class = class_physical severity failure;
    report "rt " & pv.to_value_mirror.to_physical.image;
    report "done";
    wait;
  end process;
end architecture;
