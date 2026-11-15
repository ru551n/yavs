-- Cycle mode: the testbench reads the history of signals the design drives: `'last_event`, a
-- redrive seen through `'active` and `'transaction`, `'stable`, and `'last_active` inside a
-- procedure given the signal. Their drivers stay in the event kernel (outside the subset under
-- `--cycle`, an error under `--cycle-strict`); the counter read only by value joins.
library ieee;
use ieee.std_logic_1164.all;
package hist_pkg is
  procedure show(signal x : in std_ulogic; tag : string);
end package;
package body hist_pkg is
  procedure show(signal x : in std_ulogic; tag : string) is
  begin
    report tag & " la=" & time'image(x'last_active);
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity hist_dut is
  port (
    clk : in std_ulogic;
    q, r, t, k : out std_ulogic := '0';
    c : out unsigned(7 downto 0) := x"00"
  );
end entity;
architecture a of hist_dut is
  signal n : unsigned(7 downto 0) := x"00";
begin
  pq : process (clk)
  begin
    if rising_edge(clk) then
      q <= n(1);
    end if;
  end process;
  -- The same value again at every edge: a transaction, no event.
  pr : process (clk)
  begin
    if rising_edge(clk) then
      r <= n(3);
    end if;
  end process;
  pt : process (clk)
  begin
    if rising_edge(clk) then
      t <= n(0) and n(2);
    end if;
  end process;
  pk : process (clk)
  begin
    if rising_edge(clk) then
      k <= n(2);
    end if;
  end process;
  pn : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
    end if;
  end process;
  c <= n;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.hist_pkg.all;
entity cycle_history is
end entity;
architecture a of cycle_history is
  signal clk : std_ulogic := '0';
  signal q, r, t, k : std_ulogic;
  signal c : unsigned(7 downto 0);
begin
  dut : entity work.hist_dut port map (clk => clk, q => q, r => r, t => t, k => k, c => c);
  main : process
  begin
    for i in 1 to 24 loop
      clk <= '1';
      wait for 1 ns;
      report "c=" & to_hstring(c) & " q le=" & time'image(q'last_event) & " r act="
        & boolean'image(r'active) & " t st=" & boolean'image(t'stable(2 ns));
      show(k, "k");
      clk <= '0';
      wait for 4 ns;
    end loop;
    wait;
  end process;
  tr : process
  begin
    wait on r'transaction;
    report "r tr=" & std_ulogic'image(r);
  end process;
end architecture;
