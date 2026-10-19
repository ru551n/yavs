-- difftest: top=file_append_2019 std=2019
-- APPEND_MODE creates a missing file and appends to an existing one (in 2019
-- FILE_OPEN_KIND has READ_WRITE_MODE before WRITE_MODE and APPEND_MODE).
use std.textio.all;
entity file_append_2019 is end;
architecture t of file_append_2019 is
begin
  process
    file f : text;
    variable l : line;
    variable n : integer := 0;
  begin
    for i in 1 to 2 loop
      file_open(f, "append_2019.txt", append_mode);
      write(l, string'("line ") & integer'image(i));
      writeline(f, l);
      file_close(f);
    end loop;
    file_open(f, "append_2019.txt", read_mode);
    while not endfile(f) loop
      readline(f, l);
      n := n + 1;
      report l.all;
    end loop;
    file_close(f);
    assert n = 2 severity failure;
    report "done";
    wait;
  end process;
end;
