-- difftest: top=tb_plain_copies std=2008
-- Concurrent assignments that only copy a signal (#529) and a process with a long static
-- sensitivity list (#530): values, deltas, events, 'last_value, resolution and
-- 'transaction must be those of the equivalent processes.
library ieee;
use ieee.std_logic_1164.all;

entity tb_plain_copies is end;

architecture t of tb_plain_copies is
  type rec is record
    a : std_ulogic_vector(7 downto 0);
    b : integer;
    c : std_ulogic;
    d : boolean;
  end record;
  type recs is array (0 to 1) of rec;
  constant r0 : rec := ((others => '0'), 0, '0', false);
  signal r1, r2 : rec := r0;
  signal ra : recs := (others => r0);
  signal v1 : std_ulogic_vector(15 downto 0) := (others => '0');
  signal v2 : std_ulogic_vector(0 to 7);
  signal v3 : std_logic_vector(7 downto 0);
  signal i1, i2 : integer := 5;
  signal s1, s2 : std_logic;
  signal clk : std_ulogic := '0';
  signal n, nt : natural := 0;
begin
  r2 <= r1;
  ra(1) <= r2;
  v2 <= v1(11 downto 4);
  v3 <= v1(7 downto 0);
  v3 <= v1(15 downto 8);
  i2 <= transport i1;
  s1 <= clk;
  s2 <= s1;

  watch : process (r2, ra, v2, v3, i2, s2)
  begin
    n <= n + 1;
    report "r2.a=" & to_string(r2.a) & " r2.b=" & integer'image(r2.b)
      & " r2.c=" & std_ulogic'image(r2.c) & " r2.d=" & boolean'image(r2.d)
      & " ra1.a=" & to_string(ra(1).a) & " ra0.b=" & integer'image(ra(0).b)
      & " v2=" & to_string(v2) & " v3=" & to_string(v3)
      & " i2=" & integer'image(i2) & " s2=" & std_logic'image(s2)
      & " ev=" & boolean'image(r2'event) & boolean'image(v2'event) & boolean'image(i2'event)
      & " act=" & boolean'image(i2'active) & " lv=" & to_string(v2'last_value);
  end process;

  -- Counted, not reported: same-cycle reports of two processes have no defined order.
  trans : process (i2'transaction, s2'transaction)
    variable k : natural := 0;
  begin
    k := k + 1;
    nt <= k;
  end process;

  stim : process
  begin
    wait for 1 ns;
    r1.a <= x"5a";
    r1.b <= 7;
    wait for 1 ns;
    v1 <= x"1234";
    i1 <= 9;
    wait for 1 ns;
    -- The same values again: transactions, no events.
    v1 <= x"1234";
    i1 <= 9;
    clk <= '1';
    wait for 1 ns;
    r1.c <= '1';
    r1.d <= true;
    v1 <= x"00f0";
    wait for 1 ns;
    i1 <= -1;
    clk <= '0';
    wait for 1 ns;
    assert r2 = r1 report "r2 /= r1" severity failure;
    assert ra(1) = r1 report "ra(1) /= r1" severity failure;
    assert v2 = "00001111" report "v2" severity failure;
    assert v3 = "XXXX0000" report "v3" severity failure;
    assert i2 = -1 report "i2" severity failure;
    report "done n=" & integer'image(n) & " nt=" & integer'image(nt);
    wait;
  end process;
end;
