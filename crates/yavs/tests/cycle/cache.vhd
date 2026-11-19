-- The compilation cache of cycle mode: a counter of width `W` and an integer accumulator with a
-- range check (so `YAVS_CYCLE_CHECKS=off` changes the code) in the region, a testbench that
-- prints both. `STEP` is the line the test changes in the source.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_cache_dut is
  generic (W : positive);
  port (
    clk : in std_ulogic;
    cnt : out unsigned(W - 1 downto 0);
    acc : out natural
  );
end entity;

architecture rtl of cycle_cache_dut is
  constant STEP : natural := 3;
  signal c : unsigned(W - 1 downto 0) := (others => '0');
  signal a : natural range 0 to 999 := 0;
begin
  count : process (clk)
  begin
    if rising_edge(clk) then
      c <= c + STEP;
      a <= (a + to_integer(c(3 downto 0))) mod 1000;
    end if;
  end process;
  cnt <= c;
  acc <= a;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_cache is
  generic (W : positive := 8);
end entity;

architecture tb of cycle_cache is
  signal clk : std_ulogic := '0';
  signal cnt : unsigned(W - 1 downto 0);
  signal acc : natural;
begin
  dut : entity work.cycle_cache_dut generic map (W => W) port map (clk, cnt, acc);
  main : process
  begin
    for i in 1 to 100 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "cnt=" & integer'image(to_integer(cnt)) & " acc=" & integer'image(acc);
    wait;
  end process;
end architecture;
