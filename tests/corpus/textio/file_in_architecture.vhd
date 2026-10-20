-- difftest: top=file_in_architecture std=2008
-- Files declared in an architecture: opened by the declaration, opened and closed by the
-- processes, shared by two processes of the instance, one per instance.
use std.textio.all;
entity cell_fia is
  generic (name : string);
end;
architecture t of cell_fia is
  file f : text;
  file g : text open write_mode is "fia_decl_" & name & ".txt";
begin
  process
    variable l : line;
    variable s : integer;
  begin
    file_open(f, "fia_" & name & ".txt", write_mode);
    write(l, name);
    writeline(f, l);
    writeline(g, l);
    file_close(f);
    file_open(f, "fia_" & name & ".txt", read_mode);
    readline(f, l);
    report name & " read back " & l.all;
    assert endfile(f);
    file_close(f);
    wait;
  end process;
end;

use std.textio.all;
entity file_in_architecture is end;
architecture t of file_in_architecture is
  file f : text open write_mode is "fia_top.txt";
begin
  a : entity work.cell_fia generic map ("a");
  b : entity work.cell_fia generic map ("b");
  process
    variable l : line;
  begin
    write(l, string'("top"));
    writeline(f, l);
    wait for 1 ns;
    file_close(f);
    report "closed " & boolean'image(true);
    wait;
  end process;
end;
