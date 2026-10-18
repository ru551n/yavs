-- difftest: top=driving std=2008
library ieee;
use ieee.std_logic_1164.all;
entity driving is end;
architecture t of driving is
  signal s : std_logic := '0';
  signal v : std_logic_vector(1 to 3) := "000";
  signal n : integer := 0;
begin
  s <= 'H';
  process
  begin
    s <= '1';
    v <= "1Z0";
    n <= 4;
    wait for 0 ns;
    report "s=" & std_logic'image(s) & " drv=" & std_logic'image(s'driving_value)
      & " v=" & to_string(v'driving_value) & " n=" & integer'image(n'driving_value)
      & " " & boolean'image(s'driving);
    wait;
  end process;
end;
