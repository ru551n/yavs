-- yavs-test: std=2019
-- expect: E1301 18:12 no visible subprogram accepts these arguments
-- expect: E1301 19:12 no visible subprogram accepts these arguments
-- LRM 12.5
-- A reflection method called with an INTEGER where INDEX is expected, then selected from: the
-- call is reported as a mismatch, not "not an object, literal or function" for the whole name.
use std.reflection.all;
entity e is end;
architecture a of e is
  type rec is record x : integer; end record;
begin
  process
    variable v : rec;
    variable i : integer := 1;
    variable m : record_value_mirror;
  begin
    m := v'reflect.to_record;
    report m.get(i).get_subtype_mirror.simple_name;
    assert m.get(i).get_value_class = class_integer;
    wait;
  end process;
end;
