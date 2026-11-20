-- difftest: top=copy_back_range_logic_entry std=2008 exit=1 ref=hand
-- ref=hand for the message: sim1 and sim2 give this result, sim1 prints the literals (#1178). The
-- value of a logic element passed to an inout formal of subtype X01 belongs to X01 when it is
-- copied in (LRM 4.2.2.2, #1175): 'Z' does not.
library ieee;
use ieee.std_logic_1164.all;
entity copy_back_range_logic_entry is end entity;
architecture a of copy_back_range_logic_entry is
  procedure flip (variable o : inout X01) is
  begin
    o := not o;
  end procedure;
begin
  process
    variable v : std_ulogic_vector(0 to 3) := "01Z0";
    variable i : integer := 0;
  begin
    flip(v(i));
    report "v=" & to_string(v);
    i := 2;
    flip(v(i));
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
