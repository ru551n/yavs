-- difftest: top=call_path_thin std=2019
-- std.env call paths through thin subprograms, which only pass the call on: every one of them
-- is an element of the path with the line of its calling statement.
use std.env.all;
entity call_path_thin is end entity;

architecture test of call_path_thin is
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

  impure function thin1(tag : string) return natural is
  begin
    return walk(tag);
  end function;

  impure function thin2(tag : string) return natural is
  begin
    return thin1(tag);
  end function;

  procedure outer is
  begin
    report "outer sees " & integer'image(thin2("outer"));
  end procedure;
begin
  main: process
  begin
    outer;
    report "main sees " & integer'image(thin2("main"));
    wait;
  end process;
end architecture;
