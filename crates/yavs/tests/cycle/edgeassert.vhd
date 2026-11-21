-- Concurrent asserts that test a clock edge (`assert not (rising_edge(clk) and a and b)`): checks at the
-- edge on the values before it, as their equivalent processes in the normal mode; `d` changing
-- between edges, and in the delta of the edge, fires only where the normal mode fires. One of the
-- testbench reading a member's output joins too.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_edgeassert_dut is
  port (clk : in std_ulogic; v : in std_ulogic; d : in unsigned(W - 1 downto 0); q : out unsigned(W - 1 downto 0));
end entity;

architecture rtl of cycle_edgeassert_dut is
  signal n : unsigned(W - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
    end if;
  end process;

  assert not (rising_edge(clk) and v = '1' and d = n)
    report "d meets n at " & to_hstring(std_ulogic_vector(n)) severity warning;

  assert not (falling_edge(clk) and n = 3)
    report "n is 3 at the falling edge" severity note;

  assert not rising_edge(clk) or n /= 9 report "not the edge form" severity note;

  q <= n;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_edgeassert is
end entity;

architecture tb of cycle_edgeassert is
  signal clk, v : std_ulogic := '0';
  signal d, q : unsigned(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.cycle_edgeassert_dut port map (clk => clk, v => v, d => d, q => q);

  -- Of the top-level unit, reading only what members drive: a clocked member too.
  assert not (rising_edge(clk) and q = to_unsigned(1, W)) report "tb sees 1" severity note;

  main : process
  begin
    for k in 0 to 11 loop
      -- d set in the delta of the edge: the assert sees it, as the normal mode's process.
      clk <= '1';
      v <= '1';
      d <= to_unsigned(k, W) when k mod 3 = 0 else to_unsigned(k + 5, W);
      wait for 5 ns;
      clk <= '0';
      v <= '0';
      d <= to_unsigned(k + 1, W);
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;
