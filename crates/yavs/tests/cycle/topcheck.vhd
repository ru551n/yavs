-- Asserts and reports in processes of the top-level unit (A19), `W` replaced by a width: the
-- clocked ones are evaluated at the clock edge in region code (a checker failing with severity
-- error, one passing, one checking in a called procedure, one formatting values of several kinds,
-- a variable changed after its report); the concurrent assert, the combinational report and a
-- checker of a signal the testbench drives stay in the event kernel. `cycle_topstop` ends on a checker of severity failure.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_topcheck_pkg is
  type st_t is (idle, Busy, \Odd Name\);
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
use work.cycle_topcheck_pkg.all;
entity cycle_topcheck_dut is
  port (clk : in std_ulogic; q : out unsigned(3 downto 0); wide : out std_ulogic_vector(W - 1 downto 0);
        st : out st_t);
end entity;
architecture rtl of cycle_topcheck_dut is
  signal r : unsigned(3 downto 0) := (others => '0');
  signal w : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= r + 1;
      w(0) <= not w(W - 1);
      for i in 1 to W - 1 loop
        w(i) <= w(i - 1);
      end loop;
      case to_integer(r) mod 3 is
        when 0 => st <= idle;
        when 1 => st <= Busy;
        when others => st <= \Odd Name\;
      end case;
    end if;
  end process;
  q <= r;
  wide <= w;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_topcheck_pkg.all;
entity cycle_topcheck is
end entity;
architecture sim of cycle_topcheck is
  signal clk : std_ulogic := '0';
  signal q : unsigned(3 downto 0) := (others => '0');
  signal wide : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal st : st_t := idle;
  signal done : boolean := false;
  signal flag : std_ulogic;
begin
  clk <= not clk after 5 ns when not done;
  u : entity work.cycle_topcheck_dut port map (clk => clk, q => q, wide => wide, st => st);
  -- It checks a signal the testbench drives, 'U' at first: it stays in the event kernel.
  chk3 : process (clk)
  begin
    if rising_edge(clk) then
      assert not is_x(flag) report "flag unknown" severity note;
    end if;
  end process;
  chk : process (clk)
  begin
    if rising_edge(clk) then
      assert q /= 3 report "q reached 3" severity error;
    end if;
  end process;
  pass : process (clk)
  begin
    if rising_edge(clk) then
      assert q <= 15 report "never" severity failure;
    end if;
  end process;
  chk2 : process (clk)
  begin
    if rising_edge(clk) then
      check_low(q, 12);
    end if;
  end process;
  fmt : process
    variable n : integer;
  begin
    wait until rising_edge(clk);
    n := to_integer(q) - 3;
    if q(0) = '1' then
      report "fmt " & to_string(wide) & " " & to_hstring(wide) & " " & to_ostring(wide) & " "
        & std_ulogic'image(q(1)) & " " & boolean'image(q(2) = '1') & " n=" & integer'image(n)
        & " " & st_t'image(st) & " " & to_string(st) & " " & to_string(q) & ' ' & to_string(n);
    end if;
    n := n + 100;
    assert n /= 104 severity note;
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
    wait for 30 ns;
    flag <= '0';
    wait for 170 ns;
    done <= true;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_topcheck_pkg.all;
entity cycle_topstop is
end entity;
architecture sim of cycle_topstop is
  signal clk : std_ulogic := '0';
  signal q : unsigned(3 downto 0) := (others => '0');
  signal wide : std_ulogic_vector(W - 1 downto 0);
  signal st : st_t;
begin
  clk <= not clk after 5 ns;
  u : entity work.cycle_topcheck_dut port map (clk => clk, q => q, wide => wide, st => st);
  chk : process (clk)
  begin
    if rising_edge(clk) then
      report "at " & to_string(q);
      assert q /= 5 report "q reached 5" severity failure;
    end if;
  end process;
end architecture;
