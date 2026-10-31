-- Realbench testbench for Microwatt: the SoC of the upstream core_tb with the memory image (a hex file, read with textio by the
-- upstream fpga/main_bram.vhdl) as a generic. The program writes to the console model of sim_console.vhdl; the run ends at --stop-time.
library ieee;
use ieee.std_logic_1164.all;

library work;

entity microwatt_bench_tb is
    generic (
        RAM_INIT_FILE : string  := "main_ram.hex";
        MEMORY_SIZE   : natural := 384 * 1024
    );
end microwatt_bench_tb;

architecture behave of microwatt_bench_tb is
    signal clk, rst : std_logic;
    constant clk_period : time := 10 ns;
begin
    soc0: entity work.soc
        generic map (
            SIM => true,
            MEMORY_SIZE => MEMORY_SIZE,
            RAM_INIT_FILE => RAM_INIT_FILE,
            CLK_FREQ => 100000000
        )
        port map (
            rst => rst,
            system_clk => clk
        );

    clk_process: process
    begin
        clk <= '0';
        wait for clk_period / 2;
        clk <= '1';
        wait for clk_period / 2;
    end process;

    rst_process: process
    begin
        rst <= '1';
        wait for 10 * clk_period;
        rst <= '0';
        wait;
    end process;

    jtag: entity work.sim_jtag;
end behave;
