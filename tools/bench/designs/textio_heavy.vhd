-- Text I/O: format lines with write, read them back with read, compare.
use std.textio.all;

entity bench_textio_heavy is
  generic (N : natural := 200000);
end entity;

architecture sim of bench_textio_heavy is
begin
  process
    variable l    : line;
    variable sum  : integer := 0;
    variable back : integer;
    variable ch   : character;
    variable ok   : boolean;
    variable r    : real;
  begin
    for i in 1 to N loop
      write(l, i);
      write(l, ' ');
      write(l, string'("item"));
      write(l, ' ');
      write(l, real(i) / 8.0, right, 0, 3);
      read(l, back);
      read(l, ch, ok);
      assert ok and ch = ' ' severity failure;
      read(l, ch, ok);
      read(l, ch, ok);
      read(l, ch, ok);
      read(l, ch, ok);
      read(l, ch, ok);
      read(l, r, ok);
      assert ok severity failure;
      sum := (sum + back) mod 1000003;
      deallocate(l);
    end loop;
    report "textio_heavy sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
