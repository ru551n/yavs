-- Cycle mode: a register outside the region on a clock a member makes (a divider) samples a
-- combinational output of the region a delta before the normal mode's new value arrives: the
-- clock and the settled logic leave the region in one delta, so the logic stays in the event
-- kernel. Found by the fuzzer (tools/difftest/cyclefuzz.py); the register had sampled the new value.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_dclkreg is
  generic (W : positive := 8);
end entity;
architecture a of cycle_dclkreg is
  signal clk, dk : std_ulogic := '0';
  signal q, c, r, r2 : unsigned(W - 1 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;

  cnt : process (clk)
  begin
    if rising_edge(clk) then
      q <= q + 3;
      dk <= not dk;
    end if;
  end process;

  c <= q xor shift_right(q, 1);

  slow : process (dk)
  begin
    if rising_edge(dk) then
      r <= c;
      r2 <= q;
    end if;
  end process;

  stim : process
  begin
    for i in 0 to 39 loop
      wait until falling_edge(clk);
      report "q=" & to_hstring(q) & " r=" & to_hstring(r) & " r2=" & to_hstring(r2);
    end loop;
    std.env.finish;
  end process;
end architecture;
