-- difftest: top=record_pack_signals std=2008
-- Record signals whose logic fields share one kernel signal (#686): whole and field-wise
-- drives from one process (one driver for adjacent fields), a field driven by another process,
-- a slice of a field, 'event, 'last_value and 'last_event of fields,
-- processes sensitive to one field, whole reads and drives in a process, a null field, and a
-- delayed whole assignment.
library ieee;
use ieee.std_logic_1164.all;

entity record_pack_signals is
end entity;

architecture a of record_pack_signals is
  type rec is record
    a : std_logic;
    v : std_logic_vector(3 downto 0);
    b : std_ulogic;
    n : natural range 0 to 15;
    c : bit;
    w : std_logic_vector(7 downto 0);
    z : std_logic_vector(-1 downto 0);
  end record;
  constant init : rec := ('0', "0000", '0', 0, '0', x"00", (others => '0'));
  signal r, q, d, e : rec := init;
  signal a_events, v_events : natural := 0;
begin
  -- Drives r.a and r.v (adjacent leaves: one driver) and r.n; not r.b, r.c or r.w.
  drv : process
  begin
    wait for 1 ns;
    r.a <= '1';
    r.v <= "0101";
    wait for 1 ns;
    r.v(2) <= '1';
    r.n <= 9;
    wait for 1 ns;
    report "v=" & to_string(r.v) & " v'last_value=" & to_string(r.v'last_value)
      & " a=" & std_logic'image(r.a) & " n=" & integer'image(r.n);
    r.a <= '0';
    wait for 1 ns;
    report "a'last_value=" & std_logic'image(r.a'last_value)
      & " a'last_event=" & time'image(r.a'last_event)
      & " v'last_event=" & time'image(r.v'last_event);
    wait;
  end process;

  -- The other fields of r, from another process.
  other : process
  begin
    wait for 1500 ps;
    r.b <= '1';
    r.c <= '1';
    r.w <= x"A5";
    wait;
  end process;

  -- Sensitive to one field each.
  watch_a : process (r.a)
  begin
    if r.a'event then
      a_events <= a_events + 1;
    end if;
  end process;

  watch_v : process (r.v)
  begin
    if r.v'event then
      v_events <= v_events + 1;
    end if;
  end process;

  -- Whole reads and whole drives of other packs.
  q <= r;
  comb : process (r)
    variable v : rec;
  begin
    v := r;
    v.v := not r.v;
    v.c := not r.c;
    e <= v;
  end process;
  d <= q after 2 ns;

  check : process
  begin
    wait for 10 ns;
    report "q.w=" & to_string(q.w) & " q.b=" & std_ulogic'image(q.b) & " q.c=" & bit'image(q.c)
      & " d.v=" & to_string(d.v) & " d.n=" & integer'image(d.n)
      & " a_events=" & integer'image(a_events) & " v_events=" & integer'image(v_events)
      & " e.v=" & to_string(e.v) & " e.c=" & bit'image(e.c) & " e.w=" & to_string(e.w);
    assert q = r report "copy differs" severity failure;
    assert d = r report "delayed copy differs" severity failure;
    wait;
  end process;
end architecture;
