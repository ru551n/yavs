-- difftest: top=subprog_instance_overload_2019 std=2019
-- Instances of one generic function overloaded under its own name, each reflecting its
-- own parameter (`value'reflect` in the generic body), as a record-to-dictionary package
-- does (LRM 4.4, 12.3, 16.11; #28).
use std.reflection.all;
package sio19_gen_pkg is
  impure function element_names generic (type record_t) parameter (value : record_t) return string;
end package;
use std.textio.all;
package body sio19_gen_pkg is
  impure function names_of(variable value : inout value_mirror) return string is
    variable rec : record_value_mirror;
    variable sm : record_subtype_mirror;
    variable s : line;
  begin
    if value.get_value_class /= class_record then
      return "not a record";
    end if;
    rec := value.to_record;
    sm := rec.get_subtype_mirror;
    for i in 0 to sm.length - 1 loop
      if i > 0 then
        write(s, string'(","));
      end if;
      write(s, sm.element_name(i));
    end loop;
    return s.all;
  end;

  impure function element_names generic (type record_t) parameter (value : record_t) return string is
    variable mirror : value_mirror := value'reflect;
  begin
    return names_of(mirror);
  end;
end package body;

use work.sio19_gen_pkg.all;
entity subprog_instance_overload_2019 is end entity;
architecture a of subprog_instance_overload_2019 is
  type pair_t is record first : integer; second : boolean; end record;
  type triple_t is record a, b, c : bit; end record;
  function element_names is new element_names generic map (record_t => pair_t);
  function element_names is new element_names generic map (record_t => triple_t);
  function element_names is new element_names generic map (record_t => integer);
begin
  process
    variable p : pair_t := (1, true);
    variable t : triple_t := ('0', '1', '0');
  begin
    assert element_names(p) = "FIRST,SECOND" severity failure;
    assert element_names(t) = "A,B,C" severity failure;
    assert element_names(5) = "not a record" severity failure;
    report element_names(p) & " " & element_names(t) & " " & element_names(5);
    wait;
  end process;
end architecture;
