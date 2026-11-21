-- A clocked member that changes nothing and reports at every edge (A52): its edge runs are never
-- idle, so every edge reports, as in the normal mode.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_idlenote_dut is
  port (clk : in std_ulogic; q : out std_ulogic);
end entity;

architecture rtl of cycle_idlenote_dut is
  signal held : std_ulogic := '1';
begin
  hold : process (clk)
  begin
    if rising_edge(clk) then
      held <= held;
      assert held = '0' report "held is " & std_ulogic'image(held) severity note;
    end if;
  end process;
  q <= held;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_idlenote is
end entity;

architecture tb of cycle_idlenote is
  signal clk, q : std_ulogic := '0';
begin
  dut : entity work.cycle_idlenote_dut port map (clk => clk, q => q);

  main : process
  begin
    for k in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;
