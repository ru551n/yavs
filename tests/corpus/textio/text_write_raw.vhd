-- difftest: top=text_write_raw std=2008
-- The implicit WRITE and READ of a TEXT file (a file of string) carry the characters only,
-- so READLINE sees what WRITE wrote; WRITE to OUTPUT prints them.
use std.textio.all;
entity text_write_raw is end;
architecture t of text_write_raw is
begin
  process
    file f : text;
    variable l : line;
    variable s : string(1 to 3);
    variable n : natural;
  begin
    file_open(f, "raw.txt", write_mode);
    write(f, "first" & LF);
    write(f, string'("sec"));
    write(f, "ond" & LF & "xyz");
    file_close(f);
    file_open(f, "raw.txt", read_mode);
    readline(f, l);
    assert l.all = "first" report "line 1: " & l.all severity failure;
    readline(f, l);
    assert l.all = "second" report "line 2: " & l.all severity failure;
    read(f, s, n);
    assert s = "xyz" and n = 3 report "read: " & s severity failure;
    assert endfile(f) severity failure;
    file_close(f);
    write(output, "to output" & LF);
    write(output, string'("no newline, "));
    write(output, "then one" & LF);
    report "done";
    wait;
  end process;
end;
