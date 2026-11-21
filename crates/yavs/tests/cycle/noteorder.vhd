-- Reports of clocked members of two edge classes whose edges come in one delta: made class by
-- class, reported in elaboration order as the normal mode's processes of that delta run (A25).
library ieee;
use ieee.std_logic_1164.all;

entity cycle_noteorder_dut is
  port (clk_a, clk_b : in std_ulogic);
end entity;

architecture rtl of cycle_noteorder_dut is
  signal n : natural range 0 to 15 := 0;
begin
  p1 : process (clk_a)
  begin
    if rising_edge(clk_a) then
      n <= n + 1;
      report "p1 on a at " & integer'image(n);
    end if;
  end process;

  p2 : process (clk_b)
  begin
    if rising_edge(clk_b) then
      report "p2 on b at " & integer'image(n);
    end if;
  end process;

  p3 : process (clk_a)
  begin
    if rising_edge(clk_a) then
      report "p3 on a at " & integer'image(n);
    end if;
  end process;

  p4 : process (clk_b)
  begin
    if rising_edge(clk_b) then
      report "p4 on b at " & integer'image(n);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_noteorder is
end entity;

architecture tb of cycle_noteorder is
  signal clk_a, clk_b : std_ulogic := '0';
begin
  dut : entity work.cycle_noteorder_dut port map (clk_a => clk_a, clk_b => clk_b);

  clocks : process
  begin
    for k in 1 to 3 loop
      clk_a <= '1';
      clk_b <= '1';
      wait for 5 ns;
      clk_a <= '0';
      clk_b <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;
