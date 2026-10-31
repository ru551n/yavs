library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
library rtl;
library vunit_lib;
context vunit_lib.vunit_context;

entity tb_counter is
  generic (
    runner_cfg : string;
    WIDTH      : positive := 4
  );
end entity;

architecture sim of tb_counter is
  signal clk   : std_logic := '0';
  signal rst   : std_logic := '1';
  signal count : std_logic_vector(WIDTH - 1 downto 0);
  signal wrap  : std_logic;
begin
  clk <= not clk after 5 ns;

  dut : entity rtl.counter
    generic map (WIDTH => WIDTH)
    port map (clk => clk, rst => rst, count => count, wrap => wrap);

  main : process
  begin
    test_runner_setup(runner, runner_cfg);
    wait until rising_edge(clk);
    rst <= '0';
    while test_suite loop
      if run("counts") then
        wait until rising_edge(clk);
        wait until falling_edge(clk);
        check_equal(to_integer(unsigned(count)), 1);
      elsif run("wraps") then
        wait until wrap = '1';
        wait until falling_edge(clk);
        check_equal(to_integer(unsigned(count)), 0);
      end if;
    end loop;
    test_runner_cleanup(runner);
  end process;
end architecture;
