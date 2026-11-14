-- difftest: top=write_vector_line std=2008
-- WRITE of std_ulogic_vector to a line (std_logic_1164, LRM 2008 16.4): from a procedure that
-- takes the line as `inout`, in a loop that grows the line past one allocation, justified in
-- fields narrower and wider than the value, with every value, the alias BWRITE, a slice and a
-- null vector; the line is read back between writes and written to OUTPUT.
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

entity write_vector_line is end entity;

architecture test of write_vector_line is
  procedure put(l : inout line; v : std_ulogic_vector; k : natural) is
  begin
    case k mod 4 is
      when 0 => write(l, v);
      when 1 => write(l, v, right, v'length + 3);
      when 2 => write(l, v, left, v'length / 2);
      when others => bwrite(l, v, left, v'length + 1);
    end case;
    write(l, string'("|"));
  end procedure;
begin
  process
    variable l : line;
    variable v : std_ulogic_vector(0 to 99);
    variable total : natural := 0;
  begin
    for i in v'range loop
      v(i) := std_ulogic'val((i * 7 + i / 5) mod 9);
    end loop;
    for k in 0 to 39 loop
      put(l, v(k to k + k), k);
      total := total + l'length;
      if k mod 8 = 7 then
        report integer'image(l'length) & " " & l.all(1 to 20) & " " & l.all(l'length - 9 to l'length);
        writeline(output, l);
      end if;
    end loop;
    put(l, v(5 to 4), 0);
    put(l, v, 1);
    writeline(output, l);
    report "total " & integer'image(total);
    wait;
  end process;
end architecture;
