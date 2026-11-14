-- Elements of arrays of signed as operands of `+`, in a function (a native frame) and in the
-- process (frame-mode code): compiled code reads them where they are, with no temporary and copy
-- per operand (#1122).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity clift_elem_operands is
end;
architecture a of clift_elem_operands is
  type t_arr is array (natural range <>) of signed(15 downto 0);
  function reduce(src : t_arr) return t_arr is
    variable r : t_arr(0 to (src'length + 1) / 2 - 1);
  begin
    for k in r'range loop
      if 2 * k + 1 <= src'high then
        r(k) := src(2 * k) + src(2 * k + 1);
      else
        r(k) := src(2 * k);
      end if;
    end loop;
    return r;
  end;
begin
  process is
    variable t : t_arr(0 to 6) := (others => (others => '0'));
    variable l : t_arr(0 to 3) := (others => (others => '0'));
    variable s : integer := 0;
  begin
    for i in 0 to 199 loop
      for k in t'range loop
        t(k) := t(k) + to_signed(3 * k + i, 16);
      end loop;
      l := reduce(t);
      s := (s + to_integer(l(0) + l(3))) mod 1000003;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
