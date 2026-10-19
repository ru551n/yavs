-- LRM 5.5.2, 16.4: the implicit WRITE and READ of TEXT (a file of string). Whether a record
-- carries a length is implementation defined; READLINE must see what WRITE wrote.
use std.textio.all;
entity top is end entity;
architecture a of top is
begin
  process
    file f : text;
    variable l : line;
    variable s : string(1 to 3);
    variable n : natural;
  begin
    file_open(f, "case.txt", write_mode);
    write(f, "ab" & LF);
    write(f, string'("xyzw"));
    file_close(f);
    file_open(f, "case.txt", read_mode);
    readline(f, l);
    report "RES readline " & l.all;
    read(f, s, n);
    report "RES read " & s & " " & integer'image(n);
    wait;
  end process;
end architecture;
