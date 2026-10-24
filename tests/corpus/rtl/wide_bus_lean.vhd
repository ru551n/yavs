-- difftest: top=rtl_wide_bus_lean std=2008 stop=480ns ignore=order
-- (ignore=order: the child entity and the watching process report in the same delta)
-- Attributes of wide buses (more than 64 elements) in the shapes the elaborator can trace to signal
-- objects: nothing here applies 'last_event or 'last_value to a signal parameter, so the buses
-- nothing asks the history of single elements of (a, b, c) keep only the last event of each
-- block of 64 elements, and the others (d: an element's 'last_event, e: a port formal that is
-- a part of the bus, f: through an alias, g: a record field) keep every element's. Results are the
-- same either way; see wide_bus_attributes for the attributes on parameters.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity wide_bus_part is
  port (d : in std_logic_vector(63 downto 0); q : out std_logic_vector(63 downto 0));
end;

architecture t of wide_bus_part is
begin
  process (d)
  begin
    q <= d;
    report "part event=" & boolean'image(d'event) & " never=" & boolean'image(d'last_event = time'high) &
           " last_event=" & integer'image(minimum(d'last_event, 1 ms) / 1 ns);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity rtl_wide_bus_lean is end;

architecture t of rtl_wide_bus_lean is
  type rec is record
    n    : integer;
    data : std_logic_vector(99 downto 0);
  end record;
  signal a : std_logic_vector(199 downto 0);
  signal b : bit_vector(0 to 149);
  signal c : std_logic_vector(127 downto 0);
  signal d : std_logic_vector(199 downto 0);
  signal e : std_logic_vector(199 downto 0);
  signal f : std_logic_vector(199 downto 0);
  signal g : rec;
  signal eq : std_logic_vector(63 downto 0);
  alias fa : std_logic_vector(99 downto 0) is f(99 downto 0);
  signal step : integer := 0;

  function when_t(t : time) return string is
  begin
    if t = time'high then
      return "never";
    end if;
    return integer'image(t / 1 ns) & "ns";
  end function;

  function flip(c : std_logic; n : integer) return std_logic is
    constant codes : std_logic_vector(0 to 8) := "01XZHLUW-";
  begin
    if n mod 4 = 0 then
      return codes(n mod 9);
    end if;
    return not c;
  end function;

  function chosen(k, i : integer) return boolean is
  begin
    return (k * 31 + i * 17 + (k * i) mod 13) mod 11 < (k * 5) mod 11;
  end function;

  procedure drive(signal s : inout std_logic_vector(199 downto 0); k : in integer) is
    variable v : std_logic_vector(199 downto 0);
    variable lo, hi : integer;
  begin
    case k mod 3 is
      when 0 =>
        v := s;
        for i in v'range loop
          if chosen(k, i) then v(i) := flip(v(i), k + i); end if;
        end loop;
        s <= v;
      when 1 =>
        for i in 0 to 199 loop
          if chosen(k, i) then s(i) <= flip(s(i), k + i); end if;
        end loop;
      when others =>
        lo := (k * 37) mod 150;
        hi := lo + (k * 11) mod 50;
        v := s;
        for i in lo to hi loop
          if chosen(k, i) then v(i) := flip(v(i), k + i); end if;
        end loop;
        s(hi downto lo) <= v(hi downto lo);
    end case;
  end procedure;

  signal a_st, a_qu, a_sl : boolean;
  signal a_tr : bit;
  signal a_dl : std_logic_vector(199 downto 0);
begin
  stim : process
  begin
    wait for 2 ns;
    a <= (others => '0');
    d <= (others => '0');
    e <= (others => '0');
    f <= (others => '0');
    g <= (n => 0, data => (others => '0'));
    wait for 8 ns;
    for k in 1 to 40 loop
      wait for 8 ns;
      step <= k;
      drive(a, k);
      drive(d, k + 1);
      drive(e, k + 2);
      drive(f, k + 3);
      g.n <= k;
      g.data(k + 20 downto k + 10) <= (others => '1');
      if k mod 2 = 0 then
        g.data(60 downto 50) <= (others => '0');
      end if;
      wait for 2 ns;
      if k mod 7 = 0 then
        a(5 + k) <= '1';
        d(130 + k) <= 'X';
      end if;
    end loop;
    wait;
  end process;

  echo : process (a)
  begin
    b <= to_bitvector(a(149 downto 0));
  end process;

  drv_even : process (step)
  begin
    if step mod 2 = 0 then
      c <= std_logic_vector(to_unsigned(step * 977, 32)) & std_logic_vector(to_unsigned(step, 32)) &
           std_logic_vector(to_unsigned(step * 31, 32)) & std_logic_vector(to_unsigned(step * 5, 32));
    else
      c <= (others => 'Z');
    end if;
  end process;

  drv_odd : process (step)
  begin
    if step mod 2 = 1 then
      c <= std_logic_vector(to_unsigned(step * 131, 64)) & std_logic_vector(to_unsigned(step * 7, 64));
    else
      c <= (others => 'Z');
    end if;
  end process;

  part : entity work.wide_bus_part port map (d => e(127 downto 64), q => eq);

  a_st <= a'stable(3 ns);
  a_qu <= a'quiet(3 ns);
  a_sl <= a(100 downto 90)'stable(4 ns);
  a_tr <= a'transaction;
  a_dl <= a'delayed(4 ns);

  -- In the delta of an event.
  watch : process
  begin
    wait for 5 ns;
    loop
      wait on a, b, c, d, e, f, g;
      report "w a event=" & boolean'image(a'event) & " active=" & boolean'image(a'active) &
             " le=" & when_t(a'last_event) & " la=" & when_t(a'last_active) &
             " s=" & boolean'image(a(100 downto 90)'event) & boolean'image(a(100 downto 90)'active) &
             " e5=" & boolean'image(a(5)'event) &
             " e199=" & boolean'image(a(199)'event);
      report "w a edges r=" & boolean'image(rising_edge(a(70))) & " f=" & boolean'image(falling_edge(a(70))) &
             " ev1=" & boolean'image(a(130)'event and a(130) = '1') &
             " r64=" & boolean'image(rising_edge(a(64))) & " r63=" & boolean'image(rising_edge(a(63)));
      report "w b event=" & boolean'image(b'event) & " le=" & when_t(b'last_event) &
             " e100=" & boolean'image(b(100)'event) &
             " s=" & boolean'image(b(64 to 127)'event);
      report "w c event=" & boolean'image(c'event) & " le=" & when_t(c'last_event) & " la=" & when_t(c'last_active) &
             " e70=" & boolean'image(c(70)'event);
      report "w d event=" & boolean'image(d'event) & " part_le=" & when_t(d(130 downto 60)'last_event) &
             " e70_le=" & when_t(d(70)'last_event) & " blk=" & when_t(d(100 downto 90)'last_event);
      report "w e event=" & boolean'image(e'event) & " le=" & when_t(e'last_event) & " part_le=" & when_t(e(127 downto 64)'last_event) &
             " eq=" & boolean'image(eq'event) & " eq_le=" & when_t(eq'last_event);
      report "w f event=" & boolean'image(f'event) & " fa_le=" & when_t(fa'last_event) & " fa_ev=" & boolean'image(fa'event);
      report "w g event=" & boolean'image(g'event) & " data_le=" & when_t(g.data'last_event) &
             " n_le=" & when_t(g.n'last_event) & " e=" & boolean'image(g.data'event);
    end loop;
  end process;

  -- Between events.
  sample : process
    variable prev : bit;
  begin
    wait for 6 ns;
    prev := a_tr;
    for k in 1 to 40 loop
      wait for 10 ns;
      report "s" & integer'image(k) & " stable=" & boolean'image(a_st) & " quiet=" & boolean'image(a_qu) &
             " slice=" & boolean'image(a_sl) & " tr=" & boolean'image(a_tr /= prev) &
             " delayed=" & to_string(a_dl(150 downto 100)) & " now=" & to_string(a(150 downto 100));
      prev := a_tr;
      report "s a le=" & when_t(a'last_event) & " la=" & when_t(a'last_active) & " ev=" & boolean'image(a'event) &
             " act=" & boolean'image(a'active) & " s90=" & when_t(a(100 downto 90)'last_active) &
             " e70=" & when_t(a(70 downto 70)'last_active);
      report "s b le=" & when_t(b'last_event) & " la=" & when_t(b'last_active);
      report "s c le=" & when_t(c'last_event) & " la=" & when_t(c'last_active);
      report "s d le=" & when_t(d'last_event) & " p130_60=" & when_t(d(130 downto 60)'last_event) &
             " e0=" & when_t(d(0)'last_event) & " e63=" & when_t(d(63)'last_event) & " e64=" & when_t(d(64)'last_event) &
             " e70=" & when_t(d(70)'last_event) & " e199=" & when_t(d(199)'last_event);
      report "s e le=" & when_t(e'last_event) & " part=" & when_t(e(127 downto 64)'last_event) &
             " eq=" & when_t(eq'last_event);
      report "s f le=" & when_t(f'last_event) & " fa=" & when_t(fa'last_event);
      report "s g le=" & when_t(g.data'last_event) & " n=" & when_t(g.n'last_event);
    end loop;
    wait;
  end process;
end architecture;
