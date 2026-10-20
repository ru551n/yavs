-- difftest: top=file_in_body std=2008
-- File objects declared in a protected type body: one per object, opened by a call or by the
-- declaration, used by the methods and by a private subprogram.
use std.textio.all;
package file_in_body_p is
  type t_log is protected
    procedure opn(name : string);
    procedure put(v : integer);
    procedure cls;
  end protected;
  type t_wr is protected
    procedure put(v : integer);
  end protected;
end package;
package body file_in_body_p is
  type t_log is protected body
    file f : text;
    variable l : line;
    variable cnt : integer := 0;
    procedure emit(v : integer) is
    begin
      write(l, v); writeline(f, l);
    end procedure;
    procedure opn(name : string) is
      variable st : file_open_status;
    begin
      file_open(st, f, name, write_mode);
      assert st = open_ok severity failure;
    end procedure;
    procedure put(v : integer) is
    begin
      cnt := cnt + 1;
      emit(v * 10);
    end procedure;
    procedure cls is
    begin
      file_close(f);
      report "closed after " & integer'image(cnt);
    end procedure;
  end protected body;
  type t_wr is protected body
    file g : text open write_mode is "file_in_body3.txt";
    procedure put(v : integer) is
      variable l : line;
    begin
      write(l, v); writeline(g, l);
    end procedure;
  end protected body;
end package body;
use std.textio.all;
use work.file_in_body_p.all;
entity file_in_body is end;
architecture a of file_in_body is
  shared variable a1, a2 : t_log;
  shared variable w : t_wr;
begin
  process
    file f : text;
    variable l : line;
    variable n, tot : integer := 0;
  begin
    a1.opn("file_in_body.txt");
    a2.opn("file_in_body2.txt");
    for i in 1 to 4 loop a1.put(i); end loop;
    a2.put(7);
    a1.cls; a2.cls;
    w.put(5); w.put(6);
    file_open(f, "file_in_body.txt", read_mode);
    while not endfile(f) loop
      readline(f, l); read(l, n); tot := tot + n;
    end loop;
    file_close(f);
    report "total=" & integer'image(tot);
    assert tot = 100 severity failure;
    wait;
  end process;
end;
