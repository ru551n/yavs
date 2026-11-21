-- A combinational assert that reads what depends on its own process's output (A52): `p` and `q`
-- are a loop group, `p` runs twice per settle, and only its last run, on settled values, reports.
-- The normal mode runs `p` at the delta `a` changes and again when `y` follows: its first report
-- shows the old `y`, a value cycle mode never reports.
library ieee;
use ieee.std_logic_1164.all;

entity cycle_assertloop_dut is
  port (a : in std_ulogic_vector(7 downto 0));
end entity;

architecture rtl of cycle_assertloop_dut is
  signal x, y : std_ulogic_vector(7 downto 0) := x"00";
begin
  p : process (all)
  begin
    x <= a;
    report "y is " & to_hstring(y);
  end process;

  q : process (all)
  begin
    y <= x;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity cycle_assertloop is
end entity;

architecture tb of cycle_assertloop is
  signal a : std_ulogic_vector(7 downto 0) := x"00";
begin
  dut : entity work.cycle_assertloop_dut port map (a => a);

  main : process
  begin
    wait for 10 ns;
    a <= x"11";
    wait for 10 ns;
    a <= x"22";
    wait for 10 ns;
    a <= x"22";
    wait for 10 ns;
    a <= x"A5";
    wait;
  end process;
end architecture;
