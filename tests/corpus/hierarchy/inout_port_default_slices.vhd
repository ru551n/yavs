-- difftest: top=inout_port_default_slices std=2008 ignore=order
-- `inout` vector ports whose defaults differ element by element from their actual's initial
-- value, on slices at non-zero offsets of wider signals, through two levels: one element
-- driven from 0 ns, one driven only from 1 ns (its driver starts at the port's default), the
-- others undriven (each contributes its own default element). Ascending and descending
-- ranges, so mirrored and plain element orders both occur.
-- ignore=order: the readers of the levels report in the same delta.
library ieee;
use ieee.std_logic_1164.all;

entity iopds_leaf is
  port (v : inout std_logic_vector(3 downto 0) := "HL01";
        a : inout std_logic_vector(0 to 3) := "01HL");
end;
architecture t of iopds_leaf is
begin
  d0 : v(1) <= 'Z';
  d1 : process
  begin
    wait for 1 ns;
    v(2) <= '1';
    wait;
  end process;
  e0 : a(3) <= '0';
  e1 : process
  begin
    wait for 1 ns;
    a(1) <= 'Z';
    wait;
  end process;
  rd : process (v, a)
  begin
    report "leaf v=" & to_string(v) & " a=" & to_string(a);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopds_mid is
  port (w : inout std_logic_vector(5 downto 2) := "ZZZZ";
        b : inout std_logic_vector(1 to 4) := "ZZZZ");
end;
architecture t of iopds_mid is
begin
  u : entity work.iopds_leaf port map (v => w, a => b);
end;

library ieee;
use ieee.std_logic_1164.all;

entity inout_port_default_slices is end;
architecture t of inout_port_default_slices is
  signal x : std_logic_vector(7 downto 0) := "ZZZZZZZZ";
  signal y : std_logic_vector(0 to 7) := "ZZZZZZZZ";
begin
  m : entity work.iopds_mid port map (w => x(6 downto 3), b => y(2 to 5));
  dx : x(7 downto 3) <= "0ZZZZ", "0ZZZH" after 2 ns;
  dy : y(2 to 6) <= "ZZZZ1", "ZWZZ1" after 2 ns;
  rd : process (x, y)
  begin
    report "top x=" & to_string(x) & " y=" & to_string(y);
  end process;
  check : process
  begin
    wait for 3 ns;
    report "end x=" & to_string(x) & " y=" & to_string(y);
    wait;
  end process;
end;
