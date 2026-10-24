-- difftest: top=rtl_wide_bus_attributes std=2008 stop=480ns
-- Every signal attribute on buses of more than 64 elements (several plane words): whole, per word,
-- inside a word and across words, one element, through signal parameters, after whole-vector,
-- element and slice assignments, in the delta of an event and between events, with elements that
-- never change and a resolved bus. Pins the event bookkeeping of wide signals.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity rtl_wide_bus_attributes is end;

architecture t of rtl_wide_bus_attributes is
  signal bus_a : std_logic_vector(199 downto 0);
  signal bus_b : bit_vector(0 to 149);
  signal bus_c : std_logic_vector(127 downto 0);
  signal step  : integer := 0;
  signal st_a  : boolean;
  signal qu_a  : boolean;

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

  -- 'last_value of a vector is left out: the references differ on elements that did not change
  -- in the last event, so it is read for single elements only (show1).
  procedure show(signal s : in std_logic_vector; tag : in string) is
  begin
    report tag & " event=" & boolean'image(s'event) & " active=" & boolean'image(s'active) &
           " last_event=" & when_t(s'last_event) & " last_active=" & when_t(s'last_active);
  end procedure;

  procedure show_bit(signal s : in bit_vector; tag : in string) is
  begin
    report tag & " event=" & boolean'image(s'event) & " active=" & boolean'image(s'active) &
           " last_event=" & when_t(s'last_event) & " last_active=" & when_t(s'last_active);
  end procedure;

  procedure show_bit_ev(signal s : in bit_vector; tag : in string) is
  begin
    report tag & " event=" & boolean'image(s'event) & " last_event=" & when_t(s'last_event);
  end procedure;

  procedure show1(signal s : in std_logic; tag : in string) is
  begin
    report tag & " event=" & boolean'image(s'event) & " active=" & boolean'image(s'active) &
           " last_event=" & when_t(s'last_event) & " last_active=" & when_t(s'last_active) &
           " last_value=" & std_logic'image(s'last_value) & " value=" & std_logic'image(s);
  end procedure;

  procedure show1_bit(signal s : in bit; tag : in string) is
  begin
    report tag & " event=" & boolean'image(s'event) & " last_event=" & when_t(s'last_event) &
           " last_value=" & bit'image(s'last_value);
  end procedure;
begin
  stim : process
    variable v : std_logic_vector(199 downto 0);
    variable lo, hi : integer;
  begin
    wait for 2 ns;
    bus_a <= (others => '0');
    wait for 8 ns;
    for k in 1 to 45 loop
      wait for 8 ns;
      step <= k;
      case k mod 3 is
        when 0 =>
          v := bus_a;
          for i in v'range loop
            if chosen(k, i) then
              v(i) := flip(v(i), k + i);
            end if;
          end loop;
          bus_a <= v;
        when 1 =>
          for i in 0 to 199 loop
            if chosen(k, i) then
              bus_a(i) <= flip(bus_a(i), k + i);
            end if;
          end loop;
        when others =>
          lo := (k * 37) mod 150;
          hi := lo + (k * 11) mod 50;
          v := bus_a;
          for i in lo to hi loop
            if chosen(k, i) then
              v(i) := flip(v(i), k + i);
            end if;
          end loop;
          bus_a(hi downto lo) <= v(hi downto lo);
      end case;
      wait for 2 ns;
      if k mod 7 = 0 then
        bus_a(5 + k) <= '1';
        bus_a(130 + k) <= 'X';
      end if;
    end loop;
    wait;
  end process;

  echo : process (bus_a)
  begin
    bus_b <= to_bitvector(bus_a(149 downto 0));
  end process;

  drv_even : process (step)
  begin
    if step mod 2 = 0 then
      bus_c <= std_logic_vector(to_unsigned(step * 977, 32)) & std_logic_vector(to_unsigned(step, 32)) &
               std_logic_vector(to_unsigned(step * 31, 32)) & std_logic_vector(to_unsigned(step * 5, 32));
    else
      bus_c <= (others => 'Z');
    end if;
  end process;

  drv_odd : process (step)
  begin
    if step mod 2 = 1 then
      bus_c <= std_logic_vector(to_unsigned(step * 131, 64)) & std_logic_vector(to_unsigned(step * 7, 64));
    else
      bus_c <= (others => 'Z');
    end if;
  end process;

  st_a <= bus_a'stable(3 ns);
  qu_a <= bus_a(100 downto 90)'quiet(3 ns);

  -- In the delta of an event.
  watch : process
  begin
    wait for 5 ns;
    loop
      wait on bus_a, bus_b, bus_c;
      show(bus_a, "wA");
      show(bus_a(63 downto 0), "wA63_0");
      show(bus_a(100 downto 90), "wA100_90");
      show(bus_a(130 downto 60), "wA130_60");
      show_bit_ev(bus_b, "wB");
      show_bit_ev(bus_b(64 to 127), "wB64_127");
      show(bus_c, "wC");
      show1(bus_a(5), "wA5");
      show1(bus_a(63), "wA63");
      show1(bus_a(64), "wA64");
      show1(bus_a(70), "wA70");
      show1(bus_a(127), "wA127");
      show1(bus_a(128), "wA128");
      show1(bus_a(199), "wA199");
      show1_bit(bus_b(100), "wB100");
      show1(bus_c(70), "wC70");
      report "w70 rising=" & boolean'image(rising_edge(bus_a(70))) & " falling=" & boolean'image(falling_edge(bus_a(70))) &
             " ev&1=" & boolean'image(bus_a(130)'event and bus_a(130) = '1') &
             " b100=" & boolean'image(bus_b(100)'event) & " last=" & bit'image(bus_b(100)'last_value);
    end loop;
  end process;

  -- Between events, six time units after each step's assignment settled.
  sample : process
    variable prev : bit := '0';
  begin
    wait for 6 ns;
    prev := bus_a'transaction;
    for k in 1 to 45 loop
      wait for 10 ns;
      report "s" & integer'image(k) & " stable=" & boolean'image(st_a) & " quiet=" & boolean'image(qu_a) &
             " a'stable=" & boolean'image(bus_a'stable(4 ns)) & " a'quiet=" & boolean'image(bus_a'quiet(4 ns)) &
             " s64=" & boolean'image(bus_a(127 downto 64)'stable(8 ns)) & " e70=" & boolean'image(bus_a(70)'stable(8 ns)) &
             " tr=" & boolean'image(bus_a'transaction /= prev);
      prev := bus_a'transaction;
      show(bus_a, "sA");
      show(bus_a(63 downto 0), "sA63_0");
      show(bus_a(127 downto 64), "sA127_64");
      show(bus_a(100 downto 90), "sA100_90");
      show(bus_a(199 downto 128), "sA199_128");
      show(bus_a(130 downto 60), "sA130_60");
      show_bit(bus_b, "sB");
      show_bit(bus_b(0 to 63), "sB0_63");
      show_bit(bus_b(64 to 127), "sB64_127");
      show_bit(bus_b(100 to 110), "sB100_110");
      show(bus_c, "sC");
      show(bus_c(70 downto 20), "sC70_20");
      show1(bus_a(0), "sA0e");
      show1(bus_a(5), "sA5");
      show1(bus_a(63), "sA63");
      show1(bus_a(64), "sA64");
      show1(bus_a(70), "sA70e");
      show1(bus_a(127), "sA127");
      show1(bus_a(128), "sA128");
      show1(bus_a(199), "sA199");
      show1_bit(bus_b(100), "sB100");
      show1(bus_c(70), "sC70");
      report "delayed=" & to_string(bus_a'delayed(10 ns)(150 downto 100)) & " now=" & to_string(bus_a(150 downto 100));
    end loop;
    wait;
  end process;
end architecture;
