-- yavs-test: std=2019
-- expect: none
-- LRM 4.4, 12.3, 16.11: instances of a generic function reflecting its own parameter, overloaded
-- under the generic's name in an architecture that sees the generic through a use clause (#28)
use std.reflection.all;
package gen19_pkg is
  impure function element_names generic (type record_t) parameter (value : record_t) return string;
end package;
use std.textio.all;
package body gen19_pkg is
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

use work.gen19_pkg.all;
entity tb19 is end entity;
architecture a of tb19 is
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
