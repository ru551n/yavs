-- difftest: top=call_path_branchy std=2019
-- std.env call paths through small subprograms with branches and a case, which are inlined
-- into their callers as blocks of their own: every one of them is still an element of the path
-- with the line of its calling statement.
use std.env.all;
entity call_path_branchy is end entity;

architecture test of call_path_branchy is
  type mode_t is (first, second, third);

  impure function walk(tag : string) return natural is
    variable s : call_path_vector_ptr := get_call_path;
    variable n : natural := s'length;
  begin
    for i in s'range loop
      report tag & " " & integer'image(i) & ": " & s(i).name.all & " at line " & integer'image(s(i).file_line);
    end loop;
    deallocate(s);
    return n;
  end function;

  impure function pick(mode : mode_t; tag : string) return natural is
  begin
    case mode is
      when first  => return walk(tag);
      when second => return walk(tag) + 100;
      when others => return 7;
    end case;
  end function;

  impure function gated(x : integer; mode : mode_t; tag : string) return natural is
  begin
    if x > 0 then
      return pick(mode, tag);
    else
      return 0;
    end if;
  end function;
begin
  main: process
  begin
    report "a " & integer'image(gated(1, first, "a"));
    report "b " & integer'image(gated(2, second, "b"));
    report "c " & integer'image(gated(3, third, "c"));
    report "d " & integer'image(gated(-1, first, "d"));
    wait;
  end process;
end architecture;
