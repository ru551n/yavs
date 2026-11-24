-- `--two-state`: the bounds of a subtype or a range are no values of the design, so `--init-x`
-- leaves them as written: `X01` stays `'X'` to `'1'` below the top-level unit. A two-state value
-- never equals a metavalue (A74): the one-shot "is the clock connected" check holds.
library ieee;
use ieee.std_logic_1164.all;

entity twostate_range_dut is
  port (clk : in std_logic; n : out natural; q, u : out std_logic);
end entity;

architecture a of twostate_range_dut is
begin
  assertions : process
  begin
    assert clk /= 'U' report "clock is U" severity error;
    wait;
  end process;
  u <= '1' when clk = 'U' or clk = 'X' else '0';
  process (clk)
    variable v : X01 := '0';
    variable k : natural;
  begin
    if rising_edge(clk) then
      v := '1';
      k := 0;
      for c in X01 loop
        k := k + 1;
      end loop;
      n <= k;
      q <= v;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity twostate_range is
end entity;

architecture tb of twostate_range is
  signal clk : std_logic;
  signal n : natural;
  signal q, u : std_logic;
begin
  dut : entity work.twostate_range_dut port map (clk, n, q, u);
  process
  begin
    wait for 1 ns;
    clk <= '0';
    wait for 1 ns;
    clk <= '1';
    wait for 1 ns;
    report "range n=" & integer'image(n) & " q=" & std_logic'image(q) & " u=" & std_logic'image(u);
    wait;
  end process;
end architecture;
