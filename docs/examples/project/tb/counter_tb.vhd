-- Runs the counter until it has wrapped EXPECTED_WRAPS times, then checks the time that took
library ieee;
use ieee.std_logic_1164.all;
library rtl;

entity counter_tb is
  generic (
    WIDTH          : positive := 4;
    EXPECTED_WRAPS : positive := 2;
    CLOCK_PERIOD   : time     := 10 ns
  );
end entity;

architecture sim of counter_tb is
  signal clk   : std_logic := '0';
  signal rst   : std_logic := '1';
  signal count : std_logic_vector(WIDTH - 1 downto 0);
  signal wrap  : std_logic;
begin
  clk <= not clk after CLOCK_PERIOD / 2;

  dut : entity rtl.counter
    generic map (WIDTH => WIDTH)
    port map (clk => clk, rst => rst, count => count, wrap => wrap);

  main : process
    variable wraps : natural := 0;
  begin
    wait until rising_edge(clk);
    rst <= '0';
    while wraps < EXPECTED_WRAPS loop
      wait until rising_edge(clk);
      if wrap = '1' then
        wraps := wraps + 1;
      end if;
    end loop;
    report "wrapped " & integer'image(wraps) & " times at " & time'image(now);
    assert now = CLOCK_PERIOD / 2 + (1 + EXPECTED_WRAPS * 2 ** WIDTH) * CLOCK_PERIOD
      report "unexpected time" severity failure;
    std.env.finish;
  end process;
end architecture;
