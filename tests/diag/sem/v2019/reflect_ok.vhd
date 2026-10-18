-- yavs-test: std=2019
-- expect: none
-- LRM 2019 16.2, 16.11: 'reflect on objects and type marks, mirror methods through access values
use std.reflection.all;
package p is
  type rec is record
    a : integer;
    b : bit_vector(3 downto 0);
  end record;
  impure function count(x : rec) return natural;
end package;
package body p is
  impure function count(x : rec) return natural is
    variable m : value_mirror := x'reflect;
    variable s : subtype_mirror := rec'reflect;
    variable rv : record_value_mirror;
    variable rs : record_subtype_mirror;
    variable e : value_mirror;
    variable n : natural := 0;
  begin
    if m.get_value_class /= class_record or s.get_type_class /= class_record then
      return 0;
    end if;
    rv := m.to_record;
    rs := rv.get_subtype_mirror;
    for i in 0 to rs.length - 1 loop
      e := rv.get(i);
      if e.get_subtype_mirror.simple_name /= rs.element_name(i) then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
end package body;
