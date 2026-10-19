-- difftest: top=records std=2019
-- std.reflection record mirrors (LRM 2019 16.11): elements by index and name, nested records,
-- array and enumeration elements, element subtypes, round trips.
use std.reflection.all;
entity records is end entity;

architecture test of records is
  type color is (red, green, blue);
  type inner is record
    x : integer;
    y : real;
  end record;
  type outer is record
    id : natural;
    col : color;
    pos : inner;
    tag : string(1 to 3);
    t : time;
  end record;
  constant k : outer := (5, blue, (-2, 1.5), "abc", 7 ps);
begin
  process
    variable r : outer := (1, green, (10, 2.0), "xyz", 2 ns);
    variable rm : record_subtype_mirror;
    variable rv : record_value_mirror;
    variable vm : value_mirror;
  begin
    rv := r'reflect.to_record;
    rm := rv.get_subtype_mirror;
    report "outer " & rm.simple_name & " len " & index'image(rm.length);
    for i in 0 to rm.length - 1 loop
      report "el " & index'image(i) & " " & rm.element_name(i) & " : "
        & rm.element_subtype(i).simple_name & " idx "
        & index'image(rm.element_index(rm.element_name(i)));
    end loop;
    report "id " & integer'image(rv.get("id").to_integer.value)
      & " col " & rv.get(1).to_enumeration.image
      & " x " & integer'image(rv.get("pos").to_record.get("x").to_integer.value)
      & " tag2 " & rv.get("TAG").to_array.get(2).to_enumeration.image
      & " t " & rv.get(4).to_physical.image;
    assert rv.get("pos").to_record.get(1).to_floating.value = 2.0 severity failure;
    report "pos sub " & rm.element_subtype("pos").to_record.simple_name & " "
      & rm.element_subtype("pos").to_record.element_name(1);
    vm := rv.to_value_mirror;
    assert vm.get_value_class = class_record severity failure;
    report "rt " & vm.get_subtype_mirror.simple_name & " "
      & integer'image(vm.to_record.get(0).to_integer.value);
    assert rm.to_subtype_mirror.get_type_class = class_record severity failure;
    report "k " & integer'image(k'reflect.to_record.get("pos").to_record.get("x").to_integer.value)
      & " " & k'reflect.to_record.get("col").to_enumeration.image;
    report "inner type " & inner'reflect.to_record.element_name(0) & " "
      & index'image(inner'reflect.to_record.length);
    report "done";
    wait;
  end process;
end architecture;
