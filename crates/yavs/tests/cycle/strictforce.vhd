-- `--cycle-strict` with a `force` in the design (E4116 for the forcing process, E4128 for the
-- others): a file of its own, since a force anywhere in the analysed design keeps every process out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity d_e4116 is
  port (clk, a : in std_ulogic; q, r : out std_ulogic);
end entity;
architecture rtl of d_e4116 is
begin
  f : process (a)
  begin
    if a = '1' then
      q <= force '1';
    else
      q <= release;
    end if;
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= a;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t_e4116 is
end entity;
architecture tb of t_e4116 is
  signal clk : std_ulogic := '0';
  signal a, q, r : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.d_e4116 port map (clk, a, q, r);
  stim : process
  begin
    wait for 10 ns;
    a <= '1';
    wait;
  end process;
end architecture;
