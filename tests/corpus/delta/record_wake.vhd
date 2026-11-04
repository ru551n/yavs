-- Processes on record signals: a sensitivity list naming the whole record (its fields are one
-- waiter for the whole signal), a wait on two neighbouring fields, a wait on one field, and
-- every edge form on a clock that is a record field. Each wake reports which fields had an event.
-- difftest: top=record_wake
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity record_wake is
end entity;
architecture a of record_wake is
  type rec is record
    a : std_ulogic;
    b : std_ulogic_vector(3 downto 0);
    c : std_ulogic;
    n : unsigned(7 downto 0);
  end record;
  type ctl is record
    clk : std_ulogic;
    en  : std_ulogic;
  end record;
  signal r : rec := ('0', "0000", '0', x"00");
  signal k : ctl := ('0', '0');
  signal q1, q2, q3, q4 : unsigned(7 downto 0) := x"00";
  function evs(signal s : rec) return string is
  begin
    return std_ulogic'image(s.a) & " " & to_string(s.b) & " " & std_ulogic'image(s.c) & " "
      & integer'image(to_integer(s.n)) & " ev=" & boolean'image(s.a'event) & boolean'image(s.b'event)
      & boolean'image(s.c'event) & boolean'image(s.n'event);
  end function;
begin
  whole : process (r)
  begin
    report "whole " & evs(r);
  end process;

  pair : process
  begin
    wait on r.b, r.c;
    report "pair " & evs(r);
  end process;

  one : process
  begin
    wait on r.n;
    report "one n=" & integer'image(to_integer(r.n));
  end process;

  f1 : process (k)
  begin
    if rising_edge(k.clk) then
      q1 <= q1 + 1;
    end if;
  end process;
  f2 : process (k)
  begin
    if k.clk'event and k.clk = '1' then
      q2 <= q2 + 1;
    end if;
  end process;
  f3 : process
  begin
    wait until rising_edge(k.clk) and k.en = '1';
    q3 <= q3 + 1;
  end process;
  f4 : process (k)
  begin
    if not k.clk'stable and k.clk = '1' then
      q4 <= q4 + 1;
    end if;
  end process;

  stim : process
  begin
    wait for 1 ns;
    r.a <= '1';
    wait for 1 ns;
    r.b <= "1010";
    wait for 1 ns;
    r.c <= '1';
    r.n <= x"05";
    wait for 1 ns;
    r <= ('0', "0101", '0', x"06");
    wait for 1 ns;
    r.b(0) <= '0';
    wait for 1 ns;
    r <= r;
    wait for 1 ns;
    for i in 1 to 6 loop
      k.clk <= '1';
      if i = 3 then
        k.en <= '1';
      end if;
      wait for 1 ns;
      k.clk <= '0';
      wait for 1 ns;
    end loop;
    report "q " & integer'image(to_integer(q1)) & " " & integer'image(to_integer(q2)) & " "
      & integer'image(to_integer(q3)) & " " & integer'image(to_integer(q4));
    assert q1 = 6 and q2 = 6 and q3 = 4 and q4 = 6 severity failure;
    wait;
  end process;
end architecture;
