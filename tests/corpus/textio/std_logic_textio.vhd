-- difftest: top=std_logic_textio std=2008
library ieee; use ieee.std_logic_1164.all;
use std.textio.all;
entity std_logic_textio is end;
architecture t of std_logic_textio is
begin
  process
    variable l : line;
    variable v : std_logic_vector(7 downto 0) := x"A5";
    variable r : std_logic_vector(3 downto 0);
  begin
    write(l, v); write(l, ' '); hwrite(l, v); write(l, ' '); owrite(l, std_logic_vector'("101010")); write(l, ' '); bwrite(l, v);
    writeline(output, l);
    write(l, string'("0101 ZZZZ"));
    read(l, r); report to_string(r);
    read(l, r); report to_string(r);
    report to_hstring(v) & " " & to_ostring(v) & " " & to_bstring(v);
    wait;
  end process;
end;
