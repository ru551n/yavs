-- difftest: top=record_signal_field_read std=2008
-- Reading one field of a record signal (and of an element of an array of records, a nested
-- record, an alias, a port and a signal parameter) reads that part alone: scalars of every
-- kind, vectors, a vector element, a whole sub-record and a whole element, in conditions,
-- function arguments, concatenations and sensitivity, before and after the signal changes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package record_signal_field_read_pkg is
  type inner_t is record
    flag : std_ulogic;
    bits : std_ulogic_vector(3 downto 0);
  end record;
  type rec_t is record
    b    : bit;
    c    : std_ulogic;
    v    : std_ulogic_vector(7 downto 0);
    w    : bit_vector(0 to 2);
    n    : integer range 0 to 255;
    i    : integer;
    r    : real;
    k    : boolean;
    e    : character;
    t    : time;
    sub  : inner_t;
    vv   : std_ulogic_vector(1 to 5);
  end record;
  type rec_arr_t is array (0 to 2) of rec_t;
  type rec_arr_down_t is array (2 downto 0) of rec_t;

  function sum_of(r : rec_t) return integer;
  procedure peek(signal p : in rec_t; variable seen : out integer);
end package;

package body record_signal_field_read_pkg is
  function sum_of(r : rec_t) return integer is
  begin
    return r.n + r.i;
  end function;

  procedure peek(signal p : in rec_t; variable seen : out integer) is
  begin
    seen := p.n + p.i;
    if p.c = '1' and p.sub.bits = "1010" then
      seen := seen + 1000;
    end if;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.record_signal_field_read_pkg.all;

entity record_signal_field_read_dut is
  port (
    pin  : in  rec_t;
    pout : out integer
  );
end entity;

architecture a of record_signal_field_read_dut is
begin
  pout <= pin.n + pin.i when pin.c = '1' else -1;
end architecture;

entity record_signal_field_read is
end entity;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_signal_field_read_pkg.all;

architecture a of record_signal_field_read is
  signal s    : rec_t;
  signal arr  : rec_arr_t;
  signal down : rec_arr_down_t;
  alias  al   : rec_t is s;
  signal pout : integer;
  signal evs  : integer := 0;
begin
  dut : entity work.record_signal_field_read_dut port map (pin => s, pout => pout);

  -- counts events on one field only
  process (s.v)
  begin
    evs <= evs + 1;
  end process;

  process
    variable seen : integer;
    variable cat  : std_ulogic_vector(11 downto 0);
  begin
    s.b <= '1';
    s.c <= '1';
    s.v <= x"A5";
    s.w <= "101";
    s.n <= 200;
    s.i <= -7;
    s.r <= 2.5;
    s.k <= true;
    s.e <= 'z';
    s.t <= 3 ns;
    s.sub.flag <= '1';
    s.sub.bits <= "1010";
    s.vv <= "10X1Z";
    for j in 0 to 2 loop
      arr(j).n <= 10 * j;
      arr(j).v <= std_ulogic_vector(to_unsigned(j, 8));
      arr(j).c <= '1' when j = 1 else '0';
      arr(j).sub.bits <= std_ulogic_vector(to_unsigned(j, 4));
      down(j).n <= 100 + j;
      down(j).vv <= "0000" & '1';
    end loop;
    wait for 1 ns;
    assert s.b = '1' report "b" severity failure;
    assert s.c = '1' report "c" severity failure;
    assert s.v = x"A5" report "v" severity failure;
    assert s.v(7) = '1' and s.v(0) = '1' and s.v(1) = '0' report "v elem" severity failure;
    assert s.v(7 downto 4) = x"A" report "v slice" severity failure;
    assert s.w = "101" report "w" severity failure;
    assert s.n = 200 report "n" severity failure;
    assert s.i = -7 report "i" severity failure;
    assert s.r = 2.5 report "r" severity failure;
    assert s.k report "k" severity failure;
    assert s.e = 'z' report "e" severity failure;
    assert s.t = 3 ns report "t" severity failure;
    assert s.sub.flag = '1' report "sub.flag" severity failure;
    assert s.sub.bits = "1010" report "sub.bits" severity failure;
    assert s.sub = (flag => '1', bits => "1010") report "sub" severity failure;
    assert s.vv = "10X1Z" report "vv" severity failure;
    assert s.vv'left = 1 and s.vv'right = 5 report "vv bounds" severity failure;
    assert s.v'length = 8 and s.v'left = 7 report "v bounds" severity failure;
    assert al.n = 200 and al.sub.bits = "1010" and al.v = x"A5" report "alias" severity failure;
    assert sum_of(s) = 193 report "sum_of" severity failure;
    seen := 0;
    peek(s, seen);
    assert seen = 1193 report "peek " & integer'image(seen) severity failure;
    assert pout = 193 report "port " & integer'image(pout) severity failure;
    cat := s.sub.bits & s.v;
    assert cat = "1010" & x"A5" report "cat" severity failure;
    cat := (s.c & s.sub.flag) & s.v(7 downto 0)(7 downto 0) & "00";
    assert cat(11 downto 10) = "11" report "cat2" severity failure;
    for j in 0 to 2 loop
      assert arr(j).n = 10 * j report "arr.n" severity failure;
      assert arr(j).v = std_ulogic_vector(to_unsigned(j, 8)) report "arr.v" severity failure;
      assert arr(j).sub.bits = std_ulogic_vector(to_unsigned(j, 4)) report "arr.bits"
        severity failure;
      assert (arr(j).c = '1') = (j = 1) report "arr.c" severity failure;
      assert down(j).n = 100 + j report "down.n" severity failure;
      assert down(j).vv(5) = '1' and down(j).vv(1) = '0' report "down.vv" severity failure;
    end loop;
    assert arr(1).v(0) = '1' and arr(2).v(1) = '1' report "arr elem" severity failure;
    assert arr(1).sub = (flag => 'U', bits => "0001") report "arr.sub" severity failure;
    assert arr(2).n = 20 and arr(0).n = 0 report "arr const idx" severity failure;
    assert arr(1) = arr(1) report "arr elem" severity failure;
    -- a field that changes is read again
    s.n <= 5;
    s.v <= x"01";
    wait for 1 ns;
    assert s.n = 5 and s.v = x"01" report "changed" severity failure;
    assert evs = 3 report "events " & integer'image(evs) severity failure;
    -- the same name in a wait condition
    s.c <= '0';
    wait until s.c = '0' for 5 ns;
    assert s.c = '0' report "wait" severity failure;
    report "done";
    wait;
  end process;
end architecture;
