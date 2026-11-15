-- Asserts and reports in processes of the top-level unit (A19): a clocked checker, one that
-- checks in a called procedure, a concurrent assert and a combinational report; each keeps its
-- failures under `--cycle` as in the normal mode, while the design below the top joins.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_topcheck_pkg is
  procedure check_low(signal q : in unsigned(3 downto 0); lim : natural);
end package;
package body cycle_topcheck_pkg is
  procedure check_low(signal q : in unsigned(3 downto 0); lim : natural) is
  begin
    assert to_integer(q) < lim report "q " & integer'image(to_integer(q)) & " not below "
      & integer'image(lim) severity warning;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_topcheck_dut is
  port (clk : in std_ulogic; q : out unsigned(3 downto 0));
end entity;
architecture rtl of cycle_topcheck_dut is
  signal r : unsigned(3 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= r + 1;
    end if;
  end process;
  q <= r;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_topcheck_pkg.all;
entity cycle_topcheck is
end entity;
architecture sim of cycle_topcheck is
  signal clk : std_ulogic := '0';
  signal q : unsigned(3 downto 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  u : entity work.cycle_topcheck_dut port map (clk => clk, q => q);
  chk : process (clk)
  begin
    if rising_edge(clk) then
      assert q /= 3 report "q reached 3" severity error;
    end if;
  end process;
  chk2 : process (clk)
  begin
    if rising_edge(clk) then
      check_low(q, 12);
    end if;
  end process;
  assert q /= 9 report "q is 9" severity note;
  rep : process (q)
  begin
    if q = 6 then
      report "q is 6";
    end if;
  end process;
  stop : process
  begin
    wait for 200 ns;
    done <= true;
    wait;
  end process;
end architecture;
