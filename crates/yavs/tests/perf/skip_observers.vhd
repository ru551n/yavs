-- Signals that allow skipping an assignment of the value they hold (`skip_signals`): the extra
-- lines make an observer of activity (EXTRA_DECL, EXTRA are replaced by the test).
library ieee;
use ieee.std_logic_1164.all;
entity skip_observers is end;
architecture a of skip_observers is
  signal a, b : std_logic := '0';
  signal n : integer := 0;
  --EXTRA_DECL
begin
  process begin
    for i in 0 to 9 loop
      wait for 10 ns;
      a <= '1';
      b <= '0';
      n <= 3;
    end loop;
    report "done";
    wait;
  end process;
  --EXTRA
end;
