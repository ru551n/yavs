-- `--init-unset=random` on a copy of a clock without an initial value: its drawn start against the
-- source's decides whether the copy has an edge at time 0 (`n` 4 or 3), so the analysis cycle mode
-- keeps in its cache must hold for every seed.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity ic_dut is
  port (
    clk : in std_logic;
    n, m : out unsigned(7 downto 0));
end entity;

architecture rtl of ic_dut is
  signal c2 : std_logic;
  signal cnt, k : unsigned(7 downto 0) := (others => '0');
begin
  c2 <= clk;
  -- A register on the source clock makes the copy one of a region clock.
  process (clk) begin
    if rising_edge(clk) then
      k <= k + 1;
    end if;
  end process;
  m <= k;
  process (c2) begin
    if rising_edge(c2) then
      cnt <= cnt + 1;
    end if;
  end process;
  n <= cnt;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity initclk is
end entity;

architecture tb of initclk is
  signal clk : std_logic := '1';
  signal n, m : unsigned(7 downto 0);
begin
  dut : entity work.ic_dut port map (clk, n, m);
  process begin
    for i in 1 to 3 loop
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      clk <= '1';
    end loop;
    wait for 1 ns;
    report "W=1 n=" & to_string(to_integer(n)) & " m=" & to_string(to_integer(m));
    wait;
  end process;
end architecture;
