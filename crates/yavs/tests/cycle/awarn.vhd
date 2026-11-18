-- Asserts and reports below the top-level unit, dropped in cycle mode (A41): `--cycle` warns
-- once (W4101) about those of severity error and failure, `--cycle-report` lists every one. The
-- failure fires at the 10th edge: the normal mode stops there, cycle mode runs on to the end.
-- The loop's assert and the function's (called twice) are one statement each. `assert false`
-- and a `report` always report when reached: each comes first of its severity.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_awarn_dut is
  port (clk : in std_ulogic; q : out unsigned(7 downto 0));
end entity;

architecture rtl of cycle_awarn_dut is
  signal n : unsigned(7 downto 0) := x"00";

  function inc(x : unsigned) return unsigned is
  begin
    assert x /= x"ff" report "wraps" severity error;
    return x + 1;
  end function;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      n <= inc(inc(n));
      assert n /= 20 report "the design ends its own test" severity failure;
      if n = 30 then
        assert false report "end of test" severity failure;
      end if;
      assert n < 250;
      assert n /= 251 report "explicit error" severity error;
      assert n /= 252 report "a warning" severity warning;
      report "a note";
      report "an error report" severity error;
      assert n /= 254 report "run-time" severity severity_level'val(to_integer(n(1 downto 0)));
      for i in 0 to 3 loop
        assert n(i) = n(i) report "in a loop" severity error;
      end loop;
    end if;
  end process;

  assert n /= 253 report "concurrent" severity error;
  q <= n;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_awarn is
end entity;

architecture tb of cycle_awarn is
  signal clk : std_ulogic := '0';
  signal q : unsigned(7 downto 0);
begin
  dut : entity work.cycle_awarn_dut port map (clk => clk, q => q);

  main : process
  begin
    for i in 1 to 30 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "end q=" & to_hstring(q);
    wait;
  end process;
end architecture;
