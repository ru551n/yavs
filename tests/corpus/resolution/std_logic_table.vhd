-- difftest: top=std_logic_table std=2008
library ieee; use ieee.std_logic_1164.all;
entity std_logic_table is end;
architecture t of std_logic_table is
begin
  process
    variable r : std_logic;
  begin
    for a in std_ulogic loop
      for b in std_ulogic loop
        r := resolved((a, b));
        report std_ulogic'image(a) & std_ulogic'image(b) & "=" & std_ulogic'image(r);
      end loop;
    end loop;
    wait;
  end process;
end;
