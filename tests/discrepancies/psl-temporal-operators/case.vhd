-- temporal PSL operators in assertions that hold
library ieee;
use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal clk : std_logic := '0';
  signal a, b, c : std_logic := '0';
  default clock is rising_edge(clk);
begin
  process begin
    for i in 0 to 7 loop
      a <= '1'; b <= '1'; c <= '1';
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    report "RES ran";
    wait;
  end process;
  d1: assert always (a = '1' -> next_a[1 to 3] (b = '1'));
  d2: assert always (a = '1' -> next_e[1 to 3] (b = '1'));
  d3: assert always (a = '1' -> next_event(c = '1')(b = '1'));
  d4: assert always (a = '1' -> (b = '1' until_ c = '1'));
  d5: assert always (a = '1' -> (b = '1' before_ c = '1'));
  d6: assert always (a = '1' <-> b = '1');
  d7: assert always ((a = '1' -> next (b = '1')) sync_abort (c = '0'));
end architecture;
