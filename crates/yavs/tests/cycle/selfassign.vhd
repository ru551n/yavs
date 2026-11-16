-- Cycle mode: a signal a clocked process assigns itself after an earlier assignment of all or
-- part of it in the same run keeps its value (the last assignment wins): the target is the
-- next value, the source the current one, so the copy is not a no-op. Found by the fuzzer
-- (tools/difftest/cyclefuzz.py); the region had dropped the copy.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_selfassign is
  generic (W : positive := 8);
end entity;
architecture a of cycle_selfassign is
  type rec_t is record
    v : unsigned(W - 1 downto 0);
    b : std_ulogic;
  end record;
  type mem_t is array (0 to 3) of std_ulogic_vector(W - 1 downto 0);
  signal clk, sel : std_ulogic := '0';
  signal x : unsigned(W - 1 downto 0) := (others => '0');
  signal q, q2 : unsigned(W - 1 downto 0) := (others => '0');
  signal r : rec_t := ((others => '0'), '0');
  signal m : mem_t := (others => (others => '0'));
  signal n : integer range 0 to 255 := 0;
  signal cnt : unsigned(7 downto 0) := x"00";
begin
  clk <= not clk after 5 ns;

  regs : process (clk)
  begin
    if rising_edge(clk) then
      q(W - 1 downto W / 2) <= x(W - 1 downto W / 2);
      if sel = '0' then
        q <= q;
      end if;
      q2 <= x;
      q2 <= q2;
      r.v <= x;
      r.b <= sel;
      if sel = '1' then
        r <= r;
      end if;
      m(to_integer(cnt(1 downto 0))) <= std_ulogic_vector(x);
      if cnt(2) = '1' then
        m <= m;
      end if;
      n <= to_integer(cnt);
      if sel = '0' then
        n <= n;
      end if;
      cnt <= cnt + 1;
    end if;
  end process;

  stim : process
  begin
    for i in 0 to 39 loop
      wait until falling_edge(clk);
      report "q=" & to_hstring(q) & " q2=" & to_hstring(q2) & " r=" & to_hstring(r.v) & std_ulogic'image(r.b)
        & " m=" & to_hstring(m(0)) & "," & to_hstring(m(1)) & "," & to_hstring(m(2)) & "," & to_hstring(m(3))
        & " n=" & integer'image(n);
      x <= not x when i mod 3 = 0 else x + 1;
      sel <= '1' when i mod 4 = 1 or i mod 7 = 0 else '0';
    end loop;
    std.env.finish;
  end process;
end architecture;
