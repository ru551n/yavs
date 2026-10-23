-- abort and async_abort in assertions that hold, on the whole property and inside an implication
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
  d1: assert always ((a = '1' -> next (b = '1')) abort (c = '0'));
  d2: assert always ((a = '1' -> next (b = '1')) async_abort (c = '0'));
  d3: assert always ((c = '1') -> ((next[2] (b = '1')) abort (a = '0')));
  d4: assert always ((c = '1') -> ((next[2] (b = '1')) async_abort (a = '0')));
end architecture;
