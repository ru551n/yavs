-- The run-count rule walks a combinational process only through what each signal it drives
-- depends on, against the normal mode (`W` replaced by a width): `stale` keeps a value of its
-- previous run and reads `a`, which `mux` makes from `ca`; `mux` also makes `b` from `cb`, which
-- `stale` never sees, so `ca` and `mux` stay out and `cb` joins. `count` counts the transactions
-- of `a2`, so `mux2` must run in the normal mode's deltas: the chain feeding `b2` stays out too.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runleaf is
end entity;
architecture t of cycle_runleaf is
  signal clk : std_ulogic := '0';
  signal d, q, ca, cb, a, b, y, sb : unsigned(W - 1 downto 0) := (others => '0');
  signal ca2, cb2a, cb2b, cb2, a2, b2, sb2 : unsigned(W - 1 downto 0) := (others => '0');
  signal n2 : natural := 0;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      sb <= b;
      sb2 <= b2;
    end if;
  end process;
  ca <= q + 1;
  cb <= rotate_left(q, 1) xor q;
  mux : process (all)
  begin
    a <= ca + q;
    b <= cb + 1;
  end process;
  stale : process (all)
    variable v : unsigned(W - 1 downto 0) := (others => '0');
  begin
    y <= v;
    v := q xor a;
  end process;
  ca2 <= q - 1;
  cb2a <= rotate_left(q, 1);
  cb2b <= cb2a xor q;
  cb2 <= not cb2b;
  mux2 : process (all)
  begin
    a2 <= ca2 + q;
    b2 <= cb2 xor q;
  end process;
  count : process (a2'transaction)
    variable n : natural := 0;
  begin
    n := n + 1;
    n2 <= n;
  end process;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 10 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      d <= d(W - 2 downto 0) & r(0);
      wait for 1 ns;
      clk <= '1';
      wait for 4 ns;
      clk <= '0';
      wait for 5 ns;
      report "k=" & integer'image(k) & " y=" & to_hstring(y) & " sb=" & to_hstring(sb) &
        " sb2=" & to_hstring(sb2) & " n2=" & integer'image(n2);
    end loop;
    wait;
  end process;
end architecture;
