-- difftest: top=file_roundtrip std=2008
use std.textio.all;
entity file_roundtrip is end;
architecture t of file_roundtrip is
begin
  process
    file f : text;
    variable l : line;
    variable s : integer;
    variable n : integer := 0;
    variable status : file_open_status;
  begin
    file_open(status, f, "roundtrip.txt", write_mode);
    assert status = open_ok severity failure;
    for i in 1 to 4 loop write(l, i * 100); writeline(f, l); end loop;
    file_close(f);
    file_open(f, "roundtrip.txt", read_mode);
    while not endfile(f) loop
      readline(f, l); read(l, s); n := n + s;
      report "got " & integer'image(s);
    end loop;
    file_close(f);
    report "total=" & integer'image(n);
    file_open(status, f, "does_not_exist.txt", read_mode);
    report "missing status ok=" & boolean'image(status = open_ok);
    wait;
  end process;
end;
