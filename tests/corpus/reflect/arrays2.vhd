-- difftest: top=arrays2 std=2019 ref=hand
-- std.reflection array mirrors (LRM 2019 16.11): three dimensions, unconstrained parameters,
-- arrays of records and of arrays, null arrays, index and element subtypes.
-- ref=hand: the reference simulator 1.23 normalises low and high of the second and third
-- dimensions from 0 and fails on get of a descending third index.
use std.reflection.all;
entity arrays2 is end entity;

architecture test of arrays2 is
  type cube is array (0 to 1, 1 to 2, 3 downto 2) of integer;
  type rec is record
    a : integer;
    b : bit;
  end record;
  type rec_arr is array (positive range <>) of rec;
  type row is array (0 to 2) of natural;
  type mat is array (1 to 2) of row;
  type int_vec is array (integer range <>) of integer;
  type color is (red, green, blue);
  type by_color is array (color) of integer;

  impure function describe(v : int_vec) return string is
    variable am : array_subtype_mirror := v'reflect.to_array.get_subtype_mirror;
  begin
    return am.simple_name & " " & index'image(am.left) & " " & index'image(am.right)
      & " " & index'image(am.length) & " " & boolean'image(am.ascending)
      & " first " & integer'image(v'reflect.to_array.get(index(v'left)).to_integer.value);
  end function;
begin
  process
    variable c : cube := (((1, 2), (3, 4)), ((5, 6), (7, 8)));
    variable ra : rec_arr(1 to 2) := ((1, '0'), (2, '1'));
    variable m : mat := ((1, 2, 3), (4, 5, 6));
    variable nv : int_vec(1 to 0);
    variable bc : by_color := (10, 20, 30);
    variable am : array_subtype_mirror;
    variable av : array_value_mirror;
    variable iv : index_vector(1 to 3) := (1, 2, 2);
  begin
    av := c'reflect.to_array;
    am := av.get_subtype_mirror;
    report "cube dims " & index'image(am.dimensions) & " " & am.simple_name;
    for d in dimension range 1 to 3 loop
      report "dim " & index'image(d) & " " & index'image(am.left(d)) & " "
        & index'image(am.right(d)) & " " & index'image(am.low(d)) & " "
        & index'image(am.high(d)) & " " & index'image(am.length(d)) & " "
        & boolean'image(am.ascending(d)) & " " & am.index_subtype(d).simple_name;
    end loop;
    report "c(1,2,2) " & integer'image(av.get(1, 2, 2).to_integer.value)
      & " c(0,1,3) " & integer'image(av.get(0, 1, 3).to_integer.value)
      & " iv " & integer'image(av.get(iv).to_integer.value);
    report "ra " & integer'image(ra'reflect.to_array.get(2).to_record.get("a").to_integer.value)
      & " " & ra'reflect.to_array.get(2).to_record.get("b").to_enumeration.image
      & " el " & ra'reflect.to_array.get_subtype_mirror.element_subtype.simple_name;
    report "m " & integer'image(m'reflect.to_array.get(2).to_array.get(1).to_integer.value)
      & " el " & m'reflect.to_array.get_subtype_mirror.element_subtype.simple_name
      & " ellen " & index'image(m'reflect.to_array.get_subtype_mirror.element_subtype.to_array.length);
    report "nv len " & index'image(nv'reflect.to_array.get_subtype_mirror.length)
      & " left " & index'image(nv'reflect.to_array.get_subtype_mirror.left);
    report "desc " & describe(int_vec'(5 => 50, 6 => 60, 7 => 70));
    report "desc2 " & describe(int_vec'(3 downto 1 => 9));
    report "bc " & integer'image(bc'reflect.to_array.get(1).to_integer.value)
      & " idx " & bc'reflect.to_array.get_subtype_mirror.index_subtype.simple_name
      & " left " & index'image(bc'reflect.to_array.get_subtype_mirror.left);
    assert av.to_value_mirror.get_value_class = class_array severity failure;
    assert am.to_subtype_mirror.get_type_class = class_array severity failure;
    report "row " & row'reflect.to_array.simple_name & " " & index'image(row'reflect.to_array.high);
    report "done";
    wait;
  end process;
end architecture;
