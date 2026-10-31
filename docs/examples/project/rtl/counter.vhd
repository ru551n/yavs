-- A counter that wraps at its top value and says so for one clock
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.counter_pkg.all;

entity counter is
  generic (WIDTH : positive := 4);
  port (
    clk   : in  std_logic;
    rst   : in  std_logic;
    count : out std_logic_vector(WIDTH - 1 downto 0);
    wrap  : out std_logic
  );
end entity;

architecture rtl of counter is
  signal value : unsigned(WIDTH - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      wrap <= '0';
      if rst = '1' then
        value <= (others => '0');
      elsif std_logic_vector(value) = all_ones(WIDTH) then
        value <= (others => '0');
        wrap <= '1';
      else
        value <= value + 1;
      end if;
    end if;
  end process;
  count <= std_logic_vector(value);
end architecture;
