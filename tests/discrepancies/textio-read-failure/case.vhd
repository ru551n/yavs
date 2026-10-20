use std.textio.all;
entity top is end;
architecture a of top is
begin
  process
    variable l : line;
    variable i : integer;
  begin
    l := new string'("hello");
    read(l, i);
    report "RES after a failed read without GOOD";
    wait;
  end process;
end;
