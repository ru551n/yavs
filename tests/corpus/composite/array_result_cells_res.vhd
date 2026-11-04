-- difftest: top=array_result_cells_res std=2008 ref=hand
-- ref=hand: sim1 crashes in this resolution function; sim2 and yavs agree.
-- A resolution function whose result is an array, on a resolved composite signal.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity array_result_cells_res is end;

architecture t of array_result_cells_res is
  subtype byte is std_ulogic_vector(7 downto 0);
  type byte_vec is array (natural range <>) of byte;
  function resolve_all(v : byte_vec) return byte is
    variable r : byte := x"00";
  begin
    for i in v'range loop
      for b in r'range loop
        if v(i)(b) = '1' then r(b) := '1'; end if;
      end loop;
    end loop;
    return r;
  end;
  subtype rbyte is resolve_all byte;
  signal s_res : rbyte := x"00";

begin
  s_res <= x"01";
  s_res <= x"f0";
  process
  begin
    wait for 0 ns;
    report "resolved " & to_hstring(s_res);
    wait;
  end process;
end;
