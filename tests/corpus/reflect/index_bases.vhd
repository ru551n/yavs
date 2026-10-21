-- difftest: top=index_bases std=2019
-- Index bases of std.reflection (#156, docs/decisions.md "Reflection: names, indices and
-- case"): enumeration literals and record elements count from 0, physical units from 1
-- (the primary unit), as sim1 1.23.
use std.reflection.all;
entity index_bases is end entity;

architecture test of index_bases is
  type color is (red, green, blue);
  type dist is range 0 to 1000000
    units
      mm;
      cm = 10 mm;
      m = 1000 mm;
    end units;
  type rec is record
    a : integer;
    b : boolean;
    c : color;
  end record;
begin
  process
    variable c  : color := green;
    variable d  : dist := 5 cm;
    variable r  : rec := (7, true, blue);
    variable em : enumeration_subtype_mirror;
    variable pm : physical_subtype_mirror;
    variable pv : physical_value_mirror;
    variable rm : record_subtype_mirror;
  begin
    em := c'reflect.to_enumeration.get_subtype_mirror;
    assert em.enumeration_literal(0).image = "red" severity failure;
    assert em.enumeration_literal(2).image = "blue" severity failure;
    assert em.enumeration_literal("green").pos = 1 severity failure;
    pv := d'reflect.to_physical;
    pm := pv.get_subtype_mirror;
    assert pm.units_length = 3 severity failure;
    assert pm.unit_index("mm") = 1 severity failure;
    assert pm.unit_index("m") = 3 severity failure;
    assert pm.unit_name(1) = "MM" severity failure;
    assert pm.scale(1) = 1 and pm.scale(3) = 1000 severity failure;
    assert pv.unit_index = 1 severity failure;
    rm := r'reflect.to_record.get_subtype_mirror;
    assert rm.length = 3 severity failure;
    assert rm.element_name(0) = "A" and rm.element_name(2) = "C" severity failure;
    assert rm.element_index("a") = 0 and rm.element_index("c") = 2 severity failure;
    assert r'reflect.to_record.get(0).to_integer.value = 7 severity failure;
    report "literal 0 " & em.enumeration_literal(0).image & ", unit 1 " & pm.unit_name(1)
      & ", element 0 " & rm.element_name(0);
    wait;
  end process;
end architecture;
