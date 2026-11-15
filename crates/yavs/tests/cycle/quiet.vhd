-- Cycle mode: region outputs read outside in three ways. `cnt` (registered) is read by a clocked
-- process outside the region that the kernel skips while its inputs are idle (`copy`), and at the
-- end by `main`; `nxt` (combinational) is waited on by `watch`. The quiet path (a change with no
-- event) is taken only where nothing could see the event; the results equal the normal mode's.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_quiet is
end entity;
architecture a of cycle_quiet is
  signal clk : std_ulogic := '0';
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal nxt : unsigned(7 downto 0);
  signal last : unsigned(7 downto 0) := (others => '0');
  signal seen : unsigned(7 downto 0) := (others => '0');
  signal events : natural := 0;
begin
  comb : process (cnt)
  begin
    nxt <= cnt + 1;
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= nxt;
      last <= cnt;
    end if;
  end process;
  -- Outside the region (a `wait until` process with a `'Z'` that two-state values cannot hold),
  -- idle-skipped by the kernel: it must still see every change of `cnt`.
  copy : process
  begin
    wait until rising_edge(clk);
    seen <= cnt when cnt /= 255 else (others => 'Z');
  end process;
  -- Outside the region: counts the events of `nxt`.
  watch : process (nxt)
  begin
    events <= events + 1;
  end process;
  main : process
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "cnt=" & to_string(to_integer(cnt)) & " last=" & to_string(to_integer(last))
      & " seen=" & to_string(to_integer(seen)) & " events=" & to_string(events);
    wait;
  end process;
end architecture;
