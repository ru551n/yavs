-- difftest: top=record_signals std=2008
library ieee;
use ieee.std_logic_1164.all;
entity record_signals is end;
architecture t of record_signals is
  type inner_t is record v : std_logic_vector(3 downto 0); ok : boolean; end record;
  type rec_t is record a : integer; b : std_logic; c : inner_t; end record;
  signal r : rec_t := (0, '0', ("0000", false));
  signal q : rec_t;
begin
  watch : process (r)
  begin
    report "r=" & integer'image(r.a) & " " & std_logic'image(r.b) & " " & to_string(r.c.v)
      & " " & boolean'image(r.c.ok);
  end process;
  q <= r;
  process
  begin
    wait for 1 ns;
    r <= (5, '1', ("1010", true));
    wait for 1 ns;
    r.a <= 7;
    r.c.v <= "0110";
    wait for 1 ns;
    r.c <= ("1111", false) after 1 ns;
    wait for 2 ns;
    report "q=" & integer'image(q.a) & " " & to_string(q.c.v);
    wait;
  end process;
end;
