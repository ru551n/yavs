-- A register and a block RAM macro from the vendor primitive libraries.
library ieee;
use ieee.std_logic_1164.all;
library unisim;
use unisim.vcomponents.all;
library unimacro;
use unimacro.vcomponents.all;
library vunit_lib;
context vunit_lib.vunit_context;

entity tb_vendor is
  generic (runner_cfg : string);
end entity;

architecture sim of tb_vendor is
  signal clk, clk_g, q : std_ulogic := '0';
  signal d : std_ulogic := '0';
  signal we : std_logic_vector(1 downto 0) := "00";
  signal din : std_logic_vector(15 downto 0) := X"1234";
  signal dout : std_logic_vector(15 downto 0);
  signal addr : std_logic_vector(9 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  u_bufg : BUFG port map (O => clk_g, I => clk);
  u_fdre : FDRE port map (Q => q, C => clk_g, CE => '1', D => d, R => '0');
  u_ram : BRAM_SINGLE_MACRO
    generic map (BRAM_SIZE => "18Kb", DEVICE => "7SERIES", WRITE_WIDTH => 16, READ_WIDTH => 16, DO_REG => 0)
    port map (DO => dout, ADDR => addr, CLK => clk_g, DI => din, EN => '1', REGCE => '1', RST => '0', WE => we);

  main : process
  begin
    test_runner_setup(runner, runner_cfg);
    wait for 200 ns;
    d <= '1';
    we <= "11";
    wait until rising_edge(clk);
    wait for 1 ns;
    we <= "00";
    check_equal(q, '1');
    wait until rising_edge(clk);
    wait for 1 ns;
    check_equal(dout, std_logic_vector'(X"1234"));
    test_runner_cleanup(runner);
  end process;
end architecture;
