-- The processes outside a region that may see what it exports (YAVS_CYCLE_DOWNSTREAM): a reader of
-- a member's output, one reading that reader's signal through a port, and none of the others.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_down_sub is
  port (clk : in std_logic; d : in std_logic_vector(W - 1 downto 0); q : out std_logic_vector(W - 1 downto 0));
end;
architecture a of cycle_down_sub is
begin
  reg : process (clk) begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_down_mon is
  port (s : in std_logic_vector(W - 1 downto 0));
end;
architecture a of cycle_down_mon is
begin
  chain : process begin
    wait on s;
    wait for 0 ns;
    report "s=" & to_hstring(s);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_down is end;
architecture a of cycle_down is
  signal clk, t, t2 : std_logic := '0';
  signal d, q, q2 : std_logic_vector(W - 1 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 40 ns;
  dut : entity work.cycle_down_sub port map (clk => clk, d => d, q => q);
  reader : q2 <= not q after 1 ns;
  mon : entity work.cycle_down_mon port map (s => q2);
  stim : process begin
    d <= (others => '1');
    t <= '1';
    wait for 20 ns;
    d <= (others => '0');
    wait;
  end process;
  other : t2 <= t after 1 ns;
end;
