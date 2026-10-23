-- named sequences with boolean formals, used in covers
library ieee;
use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal clk : std_logic := '0';
  signal a, b, c : std_logic := '0';
  constant pa : string := "1101001011100110";
  constant pb : string := "0110100110010101";
  constant pc : string := "0011010101001011";
  default clock is rising_edge(clk);
begin
  process begin
    for i in pa'range loop
      a <= '1' when pa(i) = '1' else '0';
      b <= '1' when pb(i) = '1' else '0';
      c <= '1' when pc(i) = '1' else '0';
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    wait;
  end process;
  sequence pair (boolean x, y) is {x; y};
  sequence trio is {pair(a = '1', b = '1'); c = '1'};
  d1: cover pair(a = '1', b = '1') report "RES pair";
  d2: cover trio report "RES trio";
end architecture;
