-- difftest: top=write_string_forms std=2008
-- WRITE of strings to a line: from constants, variables, slices and heap objects; justified
-- both ways, wider and narrower than the string; into a null line; and the line's own
-- contents appended to itself.
use std.textio.all;

entity write_string_forms is end;
architecture t of write_string_forms is
begin
  process
    variable l : line;
    variable s : string(1 to 5) := "hello";
    variable p : line;
    constant c : string := "const";
    procedure check(got : string; want : string; what : string) is
    begin
      assert got = want report what & ": got [" & got & "] want [" & want & "]" severity failure;
    end procedure;
  begin
    write(l, s);
    check(l.all, "hello", "null line");
    write(l, c);
    check(l.all, "helloconst", "append constant");
    write(l, s(2 to 4));
    check(l.all, "helloconstell", "append slice");
    write(l, s, left, 8);
    check(l.all, "helloconstellhello   ", "left justified, wider");
    write(l, s, right, 7);
    check(l.all, "helloconstellhello     hello", "right justified, wider");
    write(l, s, left, 2);
    check(l.all, "helloconstellhello     hellohello", "narrower than the string");
    deallocate(l);
    write(l, string'("ab"));
    write(l, l.all);
    check(l.all, "abab", "line appended to itself");
    write(l, l.all, right, 10);
    check(l.all, "abab      abab", "line appended to itself, justified");
    p := new string'("heap");
    write(l, p.all);
    check(l.all, "abab      abab" & "heap", "heap string");
    write(p, p.all);
    check(p.all, "heapheap", "heap string appended to itself");
    write(l, string'(""));
    check(l.all, "abab      abab" & "heap", "empty string");
    deallocate(l);
    write(l, string'(""));
    assert l /= null and l'length = 0 report "empty write creates an empty line" severity failure;
    report "write_string_forms ok";
    wait;
  end process;
end architecture;
