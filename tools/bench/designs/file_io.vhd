-- File I/O: write lines to a text file, read them back, and a file of integers.
use std.textio.all;

entity bench_file_io is
  generic (N : natural := 200000);
end entity;

architecture sim of bench_file_io is
  type int_file is file of integer;
begin
  process
    file f  : text;
    file bf : int_file;
    variable l   : line;
    variable v   : integer;
    variable sum : natural := 0;
    variable ok  : boolean;
  begin
    file_open(f, "bench_file_io.txt", write_mode);
    for i in 1 to N loop
      write(l, string'("line "));
      write(l, i);
      write(l, string'(" payload"));
      writeline(f, l);
    end loop;
    file_close(f);
    file_open(f, "bench_file_io.txt", read_mode);
    while not endfile(f) loop
      readline(f, l);
      sum := (sum + l'length) mod 1000003;
      deallocate(l);
    end loop;
    file_close(f);
    file_open(bf, "bench_file_io.bin", write_mode);
    for i in 1 to N loop
      write(bf, i);
    end loop;
    file_close(bf);
    file_open(bf, "bench_file_io.bin", read_mode);
    while not endfile(bf) loop
      read(bf, v);
      sum := (sum + v) mod 1000003;
    end loop;
    file_close(bf);
    report "file_io sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
