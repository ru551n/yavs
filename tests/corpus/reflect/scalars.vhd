-- difftest: top=scalars std=2019
-- std.reflection mirrors of scalar values and subtypes: integer, floating, physical,
-- enumeration (LRM 2019 16.11).
use std.reflection.all;
entity scalars is end entity;

architecture test of scalars is
  type state_t is (idle, Busy, done);
  subtype small is integer range -3 to 10;
  subtype down_t is integer range 9 downto 2;
  type dist is range 0 to 1000000
    units
      mm;
      cm = 10 mm;
      m = 100 cm;
    end units;
begin
  process
    variable i : small := 7;
    variable r : real := 2.5;
    variable s : state_t := busy;
    variable c : character := 'x';
    variable b : boolean := true;
    variable d : dist := 3 m;
  begin
    -- integer
    assert i'reflect.get_value_class = class_integer severity failure;
    assert i'reflect.to_integer.value = 7 severity failure;
    assert i'reflect.to_integer.image = "7" severity failure;
    assert i'reflect.get_subtype_mirror.simple_name = "SMALL" severity failure;
    assert i'reflect.get_subtype_mirror.get_type_class = class_integer severity failure;
    assert i'reflect.to_integer.get_subtype_mirror.low.value = -3 severity failure;
    assert i'reflect.to_integer.get_subtype_mirror.high.value = 10 severity failure;
    assert i'reflect.to_integer.get_subtype_mirror.length = 14 severity failure;
    assert i'reflect.to_integer.get_subtype_mirror.ascending severity failure;
    assert down_t'reflect.to_integer.left.value = 9 severity failure;
    assert down_t'reflect.to_integer.low.value = 2 severity failure;
    assert not down_t'reflect.to_integer.ascending severity failure;
    assert integer'reflect.simple_name = "INTEGER" severity failure;
    -- floating
    assert r'reflect.get_value_class = class_floating severity failure;
    assert r'reflect.to_floating.value = 2.5 severity failure;
    assert r'reflect.get_subtype_mirror.simple_name = "REAL" severity failure;
    assert real'reflect.to_floating.ascending severity failure;
    -- enumeration
    assert s'reflect.get_value_class = class_enumeration severity failure;
    assert s'reflect.to_enumeration.pos = 1 severity failure;
    assert s'reflect.to_enumeration.image = "busy" severity failure;
    assert s'reflect.get_subtype_mirror.simple_name = "STATE_T" severity failure;
    assert state_t'reflect.to_enumeration.length = 3 severity failure;
    assert state_t'reflect.to_enumeration.high.image = "done" severity failure;
    assert state_t'reflect.to_enumeration.enumeration_literal(2).pos = 2 severity failure;
    assert state_t'reflect.to_enumeration.enumeration_literal("busy").pos = 1 severity failure;
    assert c'reflect.to_enumeration.image = "'x'" severity failure;
    assert c'reflect.to_enumeration.pos = character'pos('x') severity failure;
    assert b'reflect.to_enumeration.image = "true" severity failure;
    assert b'reflect.get_subtype_mirror.simple_name = "BOOLEAN" severity failure;
    -- physical
    assert d'reflect.get_value_class = class_physical severity failure;
    assert d'reflect.to_physical.value = 3000 severity failure;
    assert d'reflect.get_subtype_mirror.simple_name = "DIST" severity failure;
    assert dist'reflect.to_physical.units_length = 3 severity failure;
    assert dist'reflect.to_physical.scale("M") = 1000 severity failure;
    assert dist'reflect.to_physical.unit_name(dist'reflect.to_physical.unit_index("cm")) = "CM"
      severity failure;
    assert dist'reflect.to_physical.high.value = 1000000 severity failure;
    -- TIME, in units of the resolution limit
    assert time'reflect.simple_name = "TIME" severity failure;
    assert delay_length'reflect.simple_name = "DELAY_LENGTH" severity failure;
    report "done";
    wait;
  end process;
end architecture;
