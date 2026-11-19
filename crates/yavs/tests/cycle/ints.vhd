-- Cycle mode: an integer and index heavy clocked process (counters, wrap-around, integer
-- arithmetic, dynamic element reads and writes), for the checks of the region code: range and
-- overflow checks of every integer assignment and operation, index checks of every dynamic index.
-- CYCLES clock cycles.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_ints is
  generic (CYCLES : natural := 2000);
end entity;
architecture a of cycle_ints is
  signal clk : std_ulogic := '0';
  signal i : integer range 0 to 31 := 0;
  signal j : integer range 0 to 31 := 5;
  signal n : natural := 0;
  signal sum : integer := 0;
  signal acc : std_ulogic_vector(31 downto 0) := (others => '0');
  signal ring : std_ulogic_vector(0 to 99) := (others => '0');
begin
  count : process (clk)
    variable k : integer range 0 to 99;
  begin
    if rising_edge(clk) then
      if i = 31 then
        i <= 0;
      else
        i <= i + 1;
      end if;
      j <= (j * 7 + 3) mod 32;
      acc(i) <= acc(j) xor acc(31 - i) xor '1';
      sum <= sum + i * 3 - j;
      n <= n + 1;
      if k = 99 then
        k := 0;
      else
        k := k + 1;
      end if;
      ring(k) <= ring(99 - k) xnor acc(i);
    end if;
  end process;
  main : process
  begin
    for c in 1 to CYCLES loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait for 1 ns;
    report "cnt=" & integer'image(n) & " sum=" & integer'image(sum) & " acc=" & to_hstring(acc)
      & " ring=" & to_hstring(ring);
    wait;
  end process;
end architecture;
