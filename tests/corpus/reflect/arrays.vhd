-- difftest: top=arrays std=2019
-- std.reflection mirrors of array values and subtypes (LRM 2019 16.11). get(i) takes the
-- LRM index; element i from the left is get(left + i) ascending, get(left - i) descending.
use std.reflection.all;
entity arrays is end entity;

architecture test of arrays is
  type int_arr is array (natural range <>) of integer;
  type grid is array (1 to 2, 0 to 2) of integer;
begin
  process
    variable bv : bit_vector(3 downto 0) := "1010";
    variable s : string(1 to 5) := "hello";
    variable ia : int_arr(2 to 4) := (20, 30, 40);
    variable g : grid := ((1, 2, 3), (4, 5, 6));
    variable bo : boolean_vector(0 to 1) := (false, true);
  begin
    assert bv'reflect.get_value_class = class_array severity failure;
    assert bv'reflect.get_subtype_mirror.simple_name = "BIT_VECTOR" severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.left(1) = 3 severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.right(1) = 0 severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.low(1) = 0 severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.high(1) = 3 severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.length(1) = 4 severity failure;
    assert not bv'reflect.to_array.get_subtype_mirror.ascending(1) severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.dimensions = 1 severity failure;
    assert bv'reflect.to_array.get(3).to_enumeration.image = "'1'" severity failure;
    assert bv'reflect.to_array.get(3).get_subtype_mirror.simple_name = "BIT" severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.element_subtype.simple_name = "BIT"
      severity failure;
    assert bv'reflect.to_array.get_subtype_mirror.index_subtype.simple_name = "NATURAL"
      severity failure;
    assert s'reflect.to_array.get(1).to_enumeration.image = "'h'" severity failure;
    assert s'reflect.to_array.get(5).to_enumeration.image = "'o'" severity failure;
    assert s'reflect.get_subtype_mirror.simple_name = "STRING" severity failure;
    assert ia'reflect.to_array.get(2).to_integer.value = 20 severity failure;
    assert ia'reflect.to_array.get(4).to_integer.value = 40 severity failure;
    assert ia'reflect.to_array.get_subtype_mirror.ascending(1) severity failure;
    assert g'reflect.to_array.get_subtype_mirror.dimensions = 2 severity failure;
    assert g'reflect.to_array.get_subtype_mirror.length(2) = 3 severity failure;
    assert bo'reflect.to_array.get(1).to_enumeration.image = "true" severity failure;
    assert bo'reflect.to_array.get(0).to_enumeration.pos = 0 severity failure;
    report "done";
    wait;
  end process;
end architecture;
