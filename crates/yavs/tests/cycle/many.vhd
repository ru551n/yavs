-- A cycle region of more functions than the compilation cache of the mode holds, when every
-- process is cut into functions of one operation (`YAVS_CYCLE_CHUNK=1`): a process per cell,
-- one clock, and a shift register along the chain. Prints a checksum.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_many_cell is
  port (clk : in std_logic; d : in std_logic; q : out std_logic);
end entity;

architecture rtl of cycle_many_cell is
  signal r : std_logic_vector(2 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= r(1 downto 0) & d;
    end if;
  end process;
  q <= r(2);
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_many is
  generic (CELLS : positive := 9000);
end entity;

architecture tb of cycle_many is
  signal clk : std_logic := '0';
  signal link : std_logic_vector(0 to CELLS) := (others => '0');
begin
  chain : for i in 0 to CELLS - 1 generate
    u : entity work.cycle_many_cell port map (clk => clk, d => link(i), q => link(i + 1));
  end generate;
  main : process
  begin
    for i in 1 to 40 loop
      link(0) <= '1';
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "link(3)=" & std_logic'image(link(3));
    wait;
  end process;
end architecture;
