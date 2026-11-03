-- difftest: top=wide_history ignore=order ref=hand
-- ignore=order: several processes report in one delta cycle, in an order the LRM leaves open.
-- ref=hand: 'last_value of one element is that element's value before its own last change (16.2.4);
-- sim1 answers per net of the vector (`last-value-composite` in the catalogue), sim2 agrees with this file.
-- Wide buses (over 64 elements) whose history is asked in every way, next to ones where nothing
-- asks it: each kernel path that keeps less for a bus must keep what these read.
library ieee;
use ieee.std_logic_1164.all;

entity wide_reader is
  port (i : in std_logic_vector(99 downto 0));
end entity;

architecture a of wide_reader is
begin
  process (i)
  begin
    if i(70)'event then
      report "port element event, last value " & std_logic'image(i(70)'last_value);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity wide_history is
end entity;

architecture t of wide_history is
  subtype bus_t is std_logic_vector(99 downto 0);
  signal bare, part, ev, lv, le, whole, viaparam, viaport, viaalias, edge : bus_t := (others => '0');
  signal copied : bus_t := (others => '0');
  alias al : std_logic_vector(7 downto 0) is viaalias(80 downto 73);

  procedure watch (signal s : in std_logic_vector; signal tick : in std_logic) is
  begin
    if s'event then
      report "param event at " & time'image(now);
    end if;
  end procedure;

  signal tick : std_logic := '0';

  function img (v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    alias x : std_logic_vector(1 to v'length) is v;
  begin
    for k in r'range loop
      r(k) := std_logic'image(x(k))(2);
    end loop;
    return r;
  end function;
begin
  u : entity work.wide_reader port map (i => viaport);

  copied <= bare;

  process (bare)
  begin
    report "bare whole " & img(bare(3 downto 0));
  end process;

  process (part(70 downto 65))
  begin
    report "bare part " & img(part(70 downto 65));
  end process;

  process (copied)
  begin
    report "copy " & img(copied(67 downto 62));
  end process;

  process (ev)
  begin
    report "ev(3) " & boolean'image(ev(3)'event) & " ev(70) " & boolean'image(ev(70)'event) & " slice " & boolean'image(ev(69 downto 64)'event);
  end process;

  process (lv)
  begin
    report "lv(3) " & std_logic'image(lv(3)'last_value) & " lv(70) " & std_logic'image(lv(70)'last_value);
  end process;

  process (le)
  begin
    report "le(70) " & time'image(le(70)'last_event) & " whole " & time'image(le'last_event);
  end process;

  process (whole)
    variable w : bus_t;
  begin
    w := whole'last_value;
    report "whole last value " & img(w(3 downto 0)) & " stable " & boolean'image(whole'stable);
  end process;

  process (viaparam)
  begin
    watch(viaparam, tick);
  end process;

  process (viaalias)
  begin
    report "alias event " & boolean'image(al'event);
  end process;

  process (edge)
  begin
    if rising_edge(edge(70)) then
      report "rising edge of element 70";
    end if;
    if falling_edge(edge(3)) then
      report "falling edge of element 3";
    end if;
  end process;

  stim : process
    procedure step (v : bus_t) is
    begin
      bare <= v;
      part <= v;
      ev <= v;
      lv <= v;
      le <= v;
      whole <= v;
      viaparam <= v;
      viaport <= v;
      viaalias <= v;
      edge <= v;
      wait for 10 ns;
    end procedure;
    variable v : bus_t := (others => '0');
  begin
    wait for 5 ns;
    step((others => '1'));
    step((others => '1'));
    v := (others => '0');
    v(70) := '1';
    step(v);
    v(3) := '1';
    v(80) := '1';
    step(v);
    step(v);
    v := (others => '1');
    v(3) := '0';
    v(70) := '0';
    step(v);
    v(99 downto 64) := (others => 'Z');
    step(v);
    wait for 10 ns;
    report "done";
    wait;
  end process;
end architecture;
