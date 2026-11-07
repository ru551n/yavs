-- Verilog cells under a VHDL testbench: CELLS instances of the module `vcell`
-- (`verilog_cells.v`) in a ring, each fed the LFSR output of the one before it, for CYCLES
-- clock cycles. Stresses Verilog process code (vector logic, adds, shifts, concatenation,
-- non-blocking assignments) and the boundary between the two languages.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity bench_verilog_cells is
  generic (CELLS : positive := 64; CYCLES : positive := 4000);
end entity;

architecture sim of bench_verilog_cells is
  component vcell is
    generic (SEED : std_logic_vector(15 downto 0));
    port (clk : in std_logic; din : in std_logic_vector(15 downto 0);
          q, sum : out std_logic_vector(15 downto 0));
  end component;
  type words is array (0 to CELLS - 1) of std_logic_vector(15 downto 0);
  signal q, sum : words;
  signal clk : std_logic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  cells_g : for i in 0 to CELLS - 1 generate
    cell : vcell
      generic map (SEED => std_logic_vector(to_unsigned(i * 977 + 1, 16)))
      port map (clk => clk, din => q((i + CELLS - 1) mod CELLS), q => q(i), sum => sum(i));
  end generate;

  process
    variable x : unsigned(15 downto 0);
  begin
    for c in 1 to CYCLES loop
      wait until rising_edge(clk);
    end loop;
    done <= true;
    wait for 1 ns;
    x := (others => '0');
    for i in 0 to CELLS - 1 loop
      x := x xor unsigned(sum(i)) xor unsigned(q(i));
    end loop;
    report "verilog_cells checksum=" & integer'image(to_integer(x));
    wait;
  end process;
end architecture;
