-- difftest: top=signal_composite_bounds std=2019
-- Signals of records and arrays of arrays whose element bounds come from the initial value
-- (LRM 6.4.2.3) or from a partially constrained subtype, and assignments, waits and
-- attributes on elements and slices of arrays of arrays (#215).
library ieee;
use ieee.std_logic_1164.all;

package scb_pkg is
  type rec is record
    v : bit_vector;
    n : integer;
  end record;
  type cell is record
    ok : boolean;
    w  : std_logic_vector;
  end record;
  type cell_vec is array (natural range <>) of cell;
  type bv_arr is array (natural range <>) of bit_vector;
  type slv_arr is array (natural range <>) of std_logic_vector;
  type int_arr is array (natural range <>) of integer;
  type int_mat is array (natural range <>) of int_arr;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.scb_pkg.all;

entity scb_sub is
  port (a : in cell_vec; o : out natural);
end entity;

architecture a of scb_sub is
begin
  process (a)
    variable n : natural;
  begin
    n := 0;
    for i in a'range loop
      if a(i).ok then
        n := n + a(i).w'length;
      end if;
    end loop;
    o <= n;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.scb_pkg.all;

entity signal_composite_bounds is
end entity;

architecture a of signal_composite_bounds is
  subtype rec4 is rec(v(0 to 3));
  subtype slv3 is slv_arr(0 to 2);
  signal r  : rec := (v => "1011", n => 5);
  signal r4 : rec4 := (v => "0000", n => 1);
  signal p  : bv_arr(0 to 2) := ("01", "10", "11");
  signal q  : slv3(open)(3 downto 0) := ("0001", "0010", "0100");
  signal m  : int_mat(1 to 2)(0 to 1) := ((1, 2), (3, 4));
  signal cs : cell_vec := ((true, "10"), (false, "01"), (true, "11"));
  signal c2 : cell_vec(0 to 1)(w(1 downto 0));
  signal o, o2 : natural;
  signal events : natural := 0;
begin
  u : entity work.scb_sub port map (a => cs, o => o);
  u2 : entity work.scb_sub port map (a => c2, o => o2);

  watch : process (q(1))
  begin
    if q(1)'event then
      events <= events + 1;
    end if;
  end process;

  main : process
  begin
    wait for 0 ns;
    report "r.v = " & to_string(r.v) & " r.v'left = " & integer'image(r.v'left)
      & " r.n = " & integer'image(r.n) & " r4.v = " & to_string(r4.v);
    report "p(1) = " & to_string(p(1)) & " q'element'length = "
      & integer'image(q'element'length) & " m(2)(1) = " & integer'image(m(2)(1));
    report "cs(2).w = " & to_string(cs(2).w) & " o = " & integer'image(o)
      & " c2(1).w'length = " & integer'image(c2(1).w'length) & " o2 = " & integer'image(o2);
    r.v <= "0110";
    r.v(0) <= '0';
    r.n <= 7;
    r4.v(1) <= '1';
    p(2) <= "00";
    p(0 to 1) <= ("11", "11");
    q(1) <= "1010";
    q(2)(0) <= '1';
    m(1) <= (8, 9);
    m(2)(0) <= 6;
    cs(2).w <= "01";
    cs(1).ok <= true;
    c2(0) <= (true, "11");
    wait for 1 ns;
    assert r.v = "0110" and r.n = 7 severity failure;
    report "r.v = " & to_string(r.v) & " r.n = " & integer'image(r.n)
      & " r4.v = " & to_string(r4.v);
    report "p = " & to_string(p(0)) & to_string(p(1)) & to_string(p(2))
      & " q = " & to_string(q(0)) & to_string(q(1)) & to_string(q(2));
    report "m = " & integer'image(m(1)(0)) & integer'image(m(1)(1))
      & integer'image(m(2)(0)) & integer'image(m(2)(1));
    report "cs(2).w = " & to_string(cs(2).w) & " o = " & integer'image(o)
      & " o2 = " & integer'image(o2) & " events = " & integer'image(events);
    q(1) <= "1011";
    q(0 to 1) <= ("1111", "1011");
    wait for 1 ns;
    report "q = " & to_string(q(0)) & to_string(q(1)) & to_string(q(2))
      & " events = " & integer'image(events);
    wait;
  end process;
end architecture;
