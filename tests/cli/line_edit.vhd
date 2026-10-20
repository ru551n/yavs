use std.textio.all;
entity line_edit is end;
architecture a of line_edit is begin
 process
  variable l, l2 : line; variable i : integer; variable c : character; variable ok : boolean;
 begin
  write(l, string'("12 abc"));
  l2 := l;
  read(l, i);
  report "i=" & integer'image(i) & " rest=[" & l.all & "]";
  read(l, c, ok);
  read(l, c, ok);
  report "c=" & c & " rest=[" & l.all & "] ok=" & boolean'image(ok);
  read(l, i, ok);
  report "ok=" & boolean'image(ok) & " rest=[" & l.all & "]";
  report "stale: " & l2.all;
  wait;
 end process;
end;
