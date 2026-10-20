-- difftest: top=file_in_subprogram std=2008
-- A file declared in a subprogram is closed, and so written out, when the subprogram returns
-- (LRM 14.6): the second subprogram reads what the first wrote, with no close call.
use std.textio.all;
entity file_in_subprogram is end;
architecture t of file_in_subprogram is
begin
  process
    procedure put is
      file f : text open write_mode is "file_in_subprogram.txt";
      variable l : line;
    begin
      write(l, string'("hello"));
      writeline(f, l);
    end procedure;
    impure function get return string is
      file f : text open read_mode is "file_in_subprogram.txt";
      variable l : line;
    begin
      readline(f, l);
      return l.all;
    end function;
  begin
    put;
    assert get = "hello" report "file not written out" severity failure;
    report "read " & get;
    wait;
  end process;
end;
