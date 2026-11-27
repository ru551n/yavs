-- Processes outside the region that count the transactions (`'transaction`) and the events
-- (`'stable`) of a signal made by logic from a register, against the normal mode (`W` replaced
-- by a width): the normal mode runs `a <= c + q` once when `q` changes and once when `c` follows,
-- so `a` gets two transactions and two events per clock. The logic stays in the event kernel,
-- the register joins.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_txcount is
end entity;
architecture t of cycle_txcount is
  signal clk : std_ulogic := '0';
  signal d, q, c, a : unsigned(W - 1 downto 0) := (others => '0');
  signal nt, ne : natural := 0;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
  c <= q + 1;
  a <= c + q;
  count_tx : process (a'transaction)
    variable n : natural := 0;
  begin
    n := n + 1;
    nt <= n;
  end process;
  count_ev : process (a'stable)
    variable n : natural := 0;
  begin
    n := n + 1;
    ne <= n;
  end process;
  main : process
  begin
    for k in 1 to 8 loop
      d <= d + 3;
      wait for 1 ns;
      clk <= '1';
      wait for 4 ns;
      clk <= '0';
      wait for 5 ns;
      report "k=" & integer'image(k) & " nt=" & integer'image(nt) & " ne=" & integer'image(ne) &
        " a=" & to_hstring(a);
    end loop;
    wait;
  end process;
end architecture;
