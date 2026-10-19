-- (from the differential corpus)
-- A recursive image function over value_mirror, as a test runner's record dictionary does:
-- every value class through get_value_class and the to_* conversions (LRM 2019 16.11).
-- ref=hand: the reference simulator 1.23 crashes on a mirror of an array of records.
use std.reflection.all;
entity walk is end entity;

architecture test of walk is
  type color is (red, green, blue);
  type pair is record
    k : color;
    v : integer;
  end record;
  type pairs is array (1 to 2) of pair;
  type top_t is record
    name : string(1 to 2);
    ps : pairs;
    r : real;
    t : time;
    flags : bit_vector(1 downto 0);
  end record;

  impure function img(variable vm : value_mirror) return string is
    variable rv : record_value_mirror;
    variable av : array_value_mirror;
    variable am : array_subtype_mirror;
    variable rm : record_subtype_mirror;
    variable rs : string(1 to 4096);
    variable n : natural := 0;
    variable e : value_mirror;
    procedure add(x : string) is begin
      rs(n + 1 to n + x'length) := x;
      n := n + x'length;
    end procedure;
  begin
    case vm.get_value_class is
      when class_enumeration => return vm.to_enumeration.image;
      when class_integer => return vm.to_integer.image;
      when class_floating => return integer'image(integer(vm.to_floating.value * 100.0));
      when class_physical => return vm.to_physical.image;
      when class_record =>
        rv := vm.to_record;
        rm := rv.get_subtype_mirror;
        add("(");
        for i in 0 to rm.length - 1 loop
          if i > 0 then add(", "); end if;
          e := rv.get(i);
          add(rm.element_name(i) & " => " & img(e));
        end loop;
        add(")");
        return rs(1 to n);
      when class_array =>
        av := vm.to_array;
        am := av.get_subtype_mirror;
        add("[");
        for i in 0 to am.length - 1 loop
          if i > 0 then add(", "); end if;
          if am.ascending then
            e := av.get(am.left + i);
          else
            e := av.get(am.left - i);
          end if;
          add(img(e));
        end loop;
        add("]");
        return rs(1 to n);
      when others => return "?";
    end case;
  end function;
begin
  process
    variable vm : value_mirror;
    variable x : top_t := ("hi", ((red, 1), (blue, -7)), 3.25, 15 ns, "10");
  begin
    vm := x'reflect;
    report img(vm);
    vm := x.ps'reflect;
    report img(vm);
    vm := x.ps(2)'reflect;
    report img(vm);
    report "done";
    wait;
  end process;
end architecture;
