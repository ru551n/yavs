-- difftest: top=scalars2 std=2019
-- std.reflection scalar mirrors not covered by reflect/scalars: integer extremes, image
-- methods, floating subtype bounds, arrays of TIME, subtype mirrors from type marks and
-- their conversions (LRM 2019 16.11).
use std.reflection.all;
entity scalars2 is end entity;

architecture test of scalars2 is
  subtype prob is real range 0.0 to 1.0;
  subtype neg is real range 0.0 downto -2.0;
  type times is array (1 to 2) of time;
  subtype byte is natural range 0 to 255;
  type idx_arr is array (byte range 3 to 4) of character;
begin
  process
    variable p : prob := 0.5;
    variable ts : times := (1 ps, 2 us);
    variable ia : idx_arr := "ok";
    variable im : integer_subtype_mirror;
    variable sm : subtype_mirror;
  begin
    im := integer'reflect.to_integer;
    report "int " & im.high.image;
    report "natural " & natural'reflect.to_integer.low.image & " positive "
      & positive'reflect.to_integer.left.image;
    assert integer'reflect.to_integer.high.value = integer'high severity failure;
    report "prob " & prob'reflect.simple_name & " "
      & boolean'image(prob'reflect.to_floating.ascending);
    assert prob'reflect.to_floating.high.value = 1.0 severity failure;
    assert neg'reflect.to_floating.left.value = 0.0 severity failure;
    assert neg'reflect.to_floating.low.value = -2.0 severity failure;
    assert not neg'reflect.to_floating.ascending severity failure;
    assert p'reflect.to_floating.get_subtype_mirror.right.value = 1.0 severity failure;
    report "ts " & ts'reflect.to_array.get(1).to_physical.image & " "
      & integer'image(ts'reflect.to_array.get(2).to_physical.value) & " "
      & ts'reflect.to_array.get_subtype_mirror.element_subtype.simple_name;
    report "ia " & ia'reflect.to_array.get_subtype_mirror.index_subtype.simple_name & " "
      & ia'reflect.to_array.get(4).to_enumeration.image & " "
      & integer'image(ia'reflect.to_array.get_subtype_mirror.index_subtype.to_integer.high.value);
    sm := byte'reflect;
    report "byte " & sm.simple_name & " " & type_class'image(sm.get_type_class) & " "
      & sm.to_integer.to_subtype_mirror.simple_name;
    sm := times'reflect;
    report "times " & type_class'image(sm.get_type_class) & " "
      & index'image(sm.to_array.length) & " " & sm.to_array.element_subtype.to_physical.simple_name;
    report "value_class " & value_class'image(ts'reflect.get_value_class) & " "
      & value_class'image(p'reflect.get_value_class);
    report "done";
    wait;
  end process;
end architecture;
