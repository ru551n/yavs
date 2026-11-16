-- Cycle mode: a logical right shift by a count known during elaboration writes every word of its
-- target, also when the target held another value before in the same run (a signal assigned
-- twice, a variable). The region wrote only the words of the bits that remain, so a shift by 64
-- or more left the high words of the earlier value. Found by the fuzzer (cyclefuzz.py).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shiftfill is
  generic (W : positive := 8);
end entity;
architecture a of cycle_shiftfill is
  signal clk : std_ulogic := '0';
  signal x, n : unsigned(W - 1 downto 0) := (others => '0');
  signal c1, c2, c3, c4, c5, q1, q2 : unsigned(W - 1 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;

  comb : process (all)
    variable v : unsigned(W - 1 downto 0);
  begin
    c1 <= x;
    c1 <= shift_right(n, 1);
    c2 <= x;
    c2 <= shift_right(n, 64);
    c3 <= x;
    c3 <= shift_right(n, 65);
    v := x;
    v := shift_right(n, W - 1);
    c4 <= v;
    c5 <= x;
    c5 <= shift_left(n, 64);
  end process;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      q1 <= x;
      q1 <= shift_right(n, 100);
      q2 <= x;
      q2 <= shift_right(x xor n, 63);
    end if;
  end process;

  stim : process
  begin
    for i in 0 to 11 loop
      wait until falling_edge(clk);
      report "c1=" & to_hstring(c1) & " c2=" & to_hstring(c2) & " c3=" & to_hstring(c3) & " c4=" & to_hstring(c4)
        & " c5=" & to_hstring(c5) & " q1=" & to_hstring(q1) & " q2=" & to_hstring(q2);
      x <= not x;
      n <= n(W - 2 downto 0) & '1' when W > 1 else not n;
    end loop;
    std.env.finish;
  end process;
end architecture;
