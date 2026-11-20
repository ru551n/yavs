-- Cycle mode: `ext` makes the asynchronous reset `arst` of `r` through combinational logic, and
-- only the register `s` reads `r`. `ext` pulses between edges at every second edge: `r` resets,
-- and `s` takes 0 at the next edge. A monitor outside prints `s` 1 ns after each edge.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_lazyrst is
end entity;
architecture a of cycle_lazyrst is
  signal clk, ext, arst, r, s : std_ulogic := '0';
begin
  arst <= ext;
  areg : process (clk, arst)
  begin
    if arst = '1' then
      r <= '0';
    elsif rising_edge(clk) then
      r <= '1';
    end if;
  end process;
  sreg : process (clk)
  begin
    if rising_edge(clk) then
      s <= r;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 6 loop
      clk <= '1';
      wait for 2 ns;
      if i mod 2 = 0 then
        ext <= '1';
      end if;
      wait for 1 ns;
      ext <= '0';
      wait for 2 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
  mon : process
  begin
    wait until rising_edge(clk);
    wait for 1 ns;
    report "s=" & to_string(s);
  end process;
end architecture;
