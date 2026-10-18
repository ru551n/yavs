-- difftest: top=write_read_line std=2008
use std.textio.all;
entity write_read_line is end;
architecture t of write_read_line is
begin
  process
    variable l : line;
    variable i : integer; variable r : real; variable b : boolean; variable c : character; variable s : string(1 to 3);
    variable t : time;
    variable ok : boolean;
  begin
    write(l, string'("int=")); write(l, 42); write(l, ' '); write(l, true); 
    write(l, ' '); write(l, 15 ns); write(l, ' '); write(l, bit_vector'("1010"));
    writeline(output, l);
    write(l, 7, right, 5); write(l, string'("ab"), left, 4); write(l, string'("|"));
    writeline(output, l);
    write(l, string'("12 -3 4.5 TRUE abc 20 ps"));
    read(l, i); read(l, i); read(l, r); read(l, b); read(l, c); read(l, s); read(l, c); read(l, t);
    report "i=" & integer'image(i) & " r=" & real'image(r) & " b=" & boolean'image(b) & " c=[" & c & "] s=" & s & " t=" & time'image(t);
    read(l, i, ok);
    report "read at end ok=" & boolean'image(ok);
    deallocate(l);
    write(l, string'("tail"));
    report "line=" & l.all;
    deallocate(l);
    wait;
  end process;
end;
