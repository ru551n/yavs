-- difftest: top=call_path std=2019
-- std.env call paths (LRM 2019 16.5): get_call_path is innermost first, names upper case,
-- lines of the calling statements; file_name, file_line and to_string of an element.
-- Directories are not printed (they depend on the checkout).
use std.env.all;
entity call_path is end entity;

architecture test of call_path is
  function ends_with(x, y : string) return boolean is
  begin
    return x'length >= y'length and x(x'right - y'length + 1 to x'right) = y;
  end function;

  impure function depth return natural is
    variable s : call_path_vector_ptr := get_call_path;
    variable n : natural;
  begin
    assert s'left = 0 severity failure;
    assert s(0).name.all = "DEPTH" and s(0).file_line = 15 severity failure;
    assert s(0).file_name.all = "call_path.vhd" severity failure;
    assert ends_with(to_string(s(0)), "call_path.vhd:15:DEPTH") severity failure;
    assert ends_with(to_string(s.all), "call_path.vhd:15:DEPTH") = false severity failure;
    report s(1).name.all & " at line " & integer'image(s(1).file_line);
    n := s'length;
    deallocate(s);
    return n;
  end function;

  procedure outer is
  begin
    report "outer sees depth " & integer'image(depth);
    report "file_line " & file_line & " in " & file_name;
  end procedure;
begin
  main: process
  begin
    outer;
    report "main sees depth " & integer'image(depth);
    assert file_line = 39 severity failure;
    assert not ends_with(file_path, "call_path.vhd") severity failure;
    wait;
  end process;
end architecture;
