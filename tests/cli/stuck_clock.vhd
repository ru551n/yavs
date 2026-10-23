-- A clock generator that stops while a register still waits for its edge.
library ieee;
use ieee.std_logic_1164.all;
entity stuck_clock is end entity;
architecture a of stuck_clock is
  signal clk, d, q : std_logic := '0';
  signal en : std_logic := '1';
begin
  gen : process
  begin
    while en = '1' loop
      clk <= not clk;
      wait for 5 ns;
      if now > 52 ns then
        en <= '0';
      end if;
    end loop;
    wait;
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end architecture;
