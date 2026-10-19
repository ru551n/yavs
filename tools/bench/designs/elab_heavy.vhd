-- Elaboration cost: a deep generate of small entities with ports, a process each and a few
-- constants; the simulation itself is one clock cycle.
library ieee;
use ieee.std_logic_1164.all;

entity bench_elab_cell is
  generic (ID : natural);
  port (clk : in std_logic; d : in std_logic; q : out std_logic);
end entity;

architecture rtl of bench_elab_cell is
  constant K : natural := ID mod 7;
  signal r : std_logic_vector(3 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= r(2 downto 0) & d;
    end if;
  end process;
  q <= r(K mod 4);
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity bench_elab_heavy is
  generic (CELLS : positive := 20000);
end entity;

architecture sim of bench_elab_heavy is
  signal clk : std_logic := '0';
  signal link : std_logic_vector(0 to CELLS) := (others => '1');
begin
  cells_g : for i in 0 to CELLS - 1 generate
    u : entity work.bench_elab_cell
      generic map (ID => i)
      port map (clk => clk, d => link(i), q => link(i + 1));
  end generate;

  process
  begin
    clk <= '1';
    wait for 5 ns;
    clk <= '0';
    wait for 5 ns;
    report "elab_heavy done";
    wait;
  end process;
end architecture;
