-- difftest: top=redrive_values
-- Several zero-delay assignments to one signal in one activation: the last one wins and the
-- waiter sees one event, or none when the value comes back. Scalars assigned twice, a default
-- then an override (back to the value held every third clock), a loop of drives to one
-- element, an integer register, record fields with the whole record, a vector with elements
-- before and after it (constant and run-time indices) and slices around a whole assignment, a
-- procedure, a resolved signal with a second driver, and two signals assigned again in opposite
-- orders, each with a process of its own waiting on it.
library ieee;
use ieee.std_logic_1164.all;

entity redrive_values is
end entity;

architecture a of redrive_values is
  type rec_t is record
    v : std_logic;
    d : std_logic_vector(3 downto 0);
    n : integer;
  end record;
  signal clk : std_logic := '0';
  signal s, t, u, w : std_logic := '0';
  signal n : integer := 0;
  signal r : rec_t := ('0', "0000", 0);
  signal v : std_logic_vector(7 downto 0) := (others => '0');
  signal sl : std_logic_vector(7 downto 0) := (others => '0');
  signal rs : std_logic := 'Z';
  signal a1, b1 : std_logic := '0';

  procedure twice(signal x : out std_logic; p, q : std_logic) is
  begin
    x <= p;
    x <= q;
  end procedure;
begin
  clkgen : process
  begin
    for i in 1 to 12 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  drv : process (clk)
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      s <= not s;
      s <= not s;
      t <= '1';
      if k mod 3 = 0 then
        t <= t;
      else
        t <= not t;
      end if;
      for i in 0 to 6 loop
        if (i + k) mod 2 = 0 then
          u <= '1';
        else
          u <= '0';
        end if;
      end loop;
      n <= k;
      n <= n + 10 * k;
      r <= (r.v, r.d, k);
      r.v <= not r.v;
      r.n <= r.n + 1;
      r.d(1) <= '1';
      r.d(1) <= r.d(0);
      r.d(0) <= not r.d(0);
      v(2) <= '1';
      v <= not v;
      v(3) <= '0';
      v(3) <= '1';
      v(k mod 8) <= '0';
      v(k mod 8) <= 'X';
      if k mod 4 = 0 then
        v(k mod 8) <= v(k mod 8);
      end if;
      sl(5 downto 2) <= "1010";
      sl <= not sl;
      sl(4 downto 3) <= sl(6 downto 5);
      if k mod 3 = 1 then
        sl(7 downto 0) <= sl;
        sl(2) <= '1';
      end if;
      twice(w, '1', not w);
      if k mod 2 = 1 then
        rs <= '1';
        rs <= 'Z';
      else
        rs <= 'Z';
        rs <= '0';
      end if;
      if k mod 2 = 0 then
        a1 <= '1';
        b1 <= '1';
        a1 <= not a1;
        b1 <= '0';
      else
        b1 <= '1';
        a1 <= '0';
        b1 <= not b1;
        a1 <= a1;
      end if;
    end if;
  end process;

  other : process (clk)
  begin
    if falling_edge(clk) then
      rs <= 'H';
    end if;
  end process;

  watch : process
  begin
    wait on s, t, u, w, n, r, v, sl, rs, a1, b1;
    report "s=" & to_string(s) & boolean'image(s'event) & " t=" & to_string(t) & boolean'image(t'event)
      & " u=" & to_string(u) & boolean'image(u'event) & " w=" & to_string(w) & boolean'image(w'event)
      & " n=" & integer'image(n) & boolean'image(n'event)
      & " r=" & to_string(r.v) & to_string(r.d) & integer'image(r.n) & boolean'image(r'event)
      & " v=" & to_string(v) & boolean'image(v'event) & " sl=" & to_string(sl) & boolean'image(sl'event)
      & " rs=" & to_string(rs) & boolean'image(rs'event)
      & " a=" & to_string(a1) & boolean'image(a1'event) & " b=" & to_string(b1) & boolean'image(b1'event)
      & " lv=" & to_string(t'last_value) & to_string(v(3)'last_value) & to_string(v(2)'last_value);
  end process;

  wa : process (a1)
  begin
    report "a1 " & to_string(a1) & " b1 " & to_string(b1) & " " & boolean'image(b1'event);
  end process;

  wb : process (b1)
  begin
    report "b1 " & to_string(b1) & " a1 " & to_string(a1) & " " & boolean'image(a1'event);
  end process;
end architecture;
