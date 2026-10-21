-- difftest: top=multidim std=2019 ref=hand
-- array_value_mirror.get on a two-dimensional array (LRM 2019 16.11). ref=hand: sim1 1.23
-- normalises the second index from 1, so get(2, 1) returns g(2, 0) there.
use std.reflection.all;
entity multidim is end entity;

architecture test of multidim is
  type grid is array (1 to 2, 0 to 2) of integer;
begin
  process
    variable g : grid := ((1, 2, 3), (4, 5, 6));
    variable iv : index_vector(1 to 2) := (2, 2);
  begin
    assert g'reflect.to_array.get_subtype_mirror.dimensions = 2 severity failure;
    assert g'reflect.to_array.get(2, 1).to_integer.value = 5 severity failure;
    assert g'reflect.to_array.get(1, 2).to_integer.value = 3 severity failure;
    assert g'reflect.to_array.get(iv).to_integer.value = 6 severity failure;
    report "done";
    wait;
  end process;
end architecture;
