-- difftest: top=tb_mux_gates std=2008
-- Concurrent conditional assignments with one condition (a code of one element, or a relation),
-- relational operators and gates with a literal operand (#786): values over all nine std_ulogic
-- values for the select and the arms, deltas, events, delays per arm, a resolved target with two
-- such drivers, an in port, vectors wider than 64 elements, records with an integer, BIT and
-- BOOLEAN, and the nearby shapes (three arms, two conditions and an expression arm, which are
-- kernel programs since #788; numeric_std's "=" as condition or as value, unaffected, a guard,
-- postponed, a rejection limit, which stay processes) must be those of the equivalent processes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity mux_inner is
  port (i, s : in std_ulogic; o : out std_ulogic);
end;

architecture a of mux_inner is
begin
  o <= i when s = '1' else '0';
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_mux_gates is end;

architecture t of tb_mux_gates is
  constant K4 : std_logic_vector(3 downto 0) := "1010";
  constant KW : std_logic_vector(69 downto 0) := (others => '1');
  type rec is record
    f : std_logic_vector(3 downto 0);
    g : std_logic;
    n : integer;
  end record;
  signal a, b, s, en : std_logic := '0';
  signal m1, m2, m3, m4, m5, m6, m7, m8, m9, mi : std_logic;
  signal mc1, mc2, mc3, mc5, mc6 : std_logic;
  signal mcv : std_logic_vector(3 downto 0);
  signal bs, ba, bc : bit := '0';
  signal bo, bl : boolean := false;
  signal bm1, bm2 : bit;
  signal bb1 : boolean;
  signal bvm : bit_vector(2 downto 0);
  signal bv1, bv2 : bit_vector(2 downto 0) := "000";
  signal v1 : std_logic_vector(3 downto 0) := "0000";
  signal v2 : std_logic_vector(0 to 3) := "0000";
  signal vm1, vm2, vm3, vm4 : std_logic_vector(3 downto 0);
  signal vm5 : std_logic_vector(1 downto 0);
  signal w1, w2 : std_logic_vector(69 downto 0) := (others => '0');
  signal wm1, wm2 : std_logic_vector(69 downto 0);
  signal u1, u2 : unsigned(3 downto 0) := "0000";
  signal um : unsigned(3 downto 0);
  signal i1, i2, im : integer := 0;
  signal r1, r2, rm : rec := (f => "0000", g => '0', n => 0);
  signal e1, e2, e3, e4, e5, e6, e7, e8, e9, e10, e11, e12, e13 : boolean;
  signal e14 : std_ulogic;
  signal g1, g2, g3, g5 : std_logic;
  signal g4, g7 : std_logic_vector(69 downto 0);
  signal g6 : boolean;
  signal g8 : bit;
  signal g9 : std_logic_vector(3 downto 0);
  signal n1, n2, n3, n4, n5, n6, n7, n8, n9 : std_logic;
begin
  u : entity work.mux_inner port map (i => a, s => s, o => mi);
  m1 <= a when s = '1' else b;
  m2 <= a when s /= '1' else b;
  m3 <= a when '0' = s else '1';
  m4 <= '0' when s = 'H' else a;
  m5 <= a when s else b;
  m6 <= a after 2 ns when s = '1' else b after 3 ns;
  m7 <= transport a when s = '1' else b after 2 ns;
  m8 <= a when s = '1' else 'Z';
  m8 <= b when s = '0' else 'Z';
  m9 <= a when s = '-' else b;
  mc1 <= '1' when v1 = v2 else '0';
  mc2 <= a when v1 = "1010" else b;
  mc3 <= a when a = b else '0';
  mcv <= v1 when w1 /= w2 else v2;
  mc5 <= '1' when K4 /= v1 else '0';
  mc6 <= a when v1(1) = '1' else b;
  bm1 <= ba when bs = '1' else bc;
  bm2 <= '1' when bo else ba;
  bb1 <= bo when bs = '1' else true;
  bvm <= bv1 when bo else bv2;
  vm1 <= v1 when s = '1' else v2;
  vm2 <= v1 when s = '1' else (others => '0');
  vm3 <= "1010" when s = '1' else v2;
  vm4 <= K4 when s = 'X' else v1;
  vm5 <= v1(3 downto 2) when s = '1' else v2(0 to 1);
  wm1 <= w1 when s = '1' else w2;
  wm2 <= w1 when s = '1' else KW;
  um <= u1 when s = '1' else u2;
  im <= i1 when s = '1' else i2;
  rm <= r1 when s = '1' else r2;
  e1 <= a = b;
  e2 <= a /= b;
  e3 <= v1 = v2;
  e4 <= v1 = "1010";
  e5 <= K4 /= v1;
  e6 <= a = '1';
  e7 <= w1 = w2;
  e8 <= w1 = KW;
  e9 <= ba = bc;
  e10 <= bo /= bl;
  e11 <= a = b after 2 ns;
  e12 <= transport a /= b after 2 ns;
  e13 <= u1 = u2;
  e14 <= a ?= b;
  g1 <= a and '1';
  g2 <= '0' or b;
  g3 <= not a;
  g4 <= w1 xor KW;
  g5 <= a nand 'H';
  g6 <= bo and true;
  g7 <= w1 and w2;
  g8 <= ba nor '1';
  g9 <= v1 or K4;
  -- Not kernel muxes or gates: still processes.
  n1 <= a when s = '1' else b when en = '1' else '0';
  n2 <= a when s = '1' and en = '1' else b;
  n3 <= a when u1 = u2 else b;
  n4 <= a when s = '1' else unaffected;
  n5 <= (a and b) when s = '1' else '0';
  blk : block (en = '1')
  begin
    n6 <= guarded a when s = '1' else b;
  end block;
  postponed n7 <= a after 1 ns when s = '1' else b after 1 ns;
  n8 <= reject 1 ns inertial a after 2 ns when s = '1' else b after 2 ns;
  n9 <= '1' and '0';

  stim : process
    function sl(k : integer) return std_logic is
    begin
      return std_logic'val(k mod 9);
    end;
  begin
    for i in 0 to 728 loop
      a <= sl(i);
      b <= sl(i / 9);
      s <= sl(i / 81);
      en <= sl(i / 3);
      bs <= bit'val(i mod 2);
      ba <= bit'val(i / 2 mod 2);
      bc <= bit'val(i / 4 mod 2);
      bo <= i mod 3 = 0;
      bl <= i mod 4 = 0;
      bv1 <= (bit'val(i mod 2), bit'val(i / 2 mod 2), '1');
      bv2 <= (bit'val(i / 4 mod 2), '1', bit'val(i mod 2));
      v1 <= (sl(i), sl(i / 9), sl(i + 4), '1');
      v2 <= (sl(i * 7), 'H', sl(i / 9), sl(i));
      if i mod 5 = 0 then
        v2 <= v1;
      end if;
      w1 <= (others => sl(i));
      w1(64) <= sl(i / 9);
      if i mod 7 = 0 then
        w2 <= w1;
      else
        w2 <= (others => sl(i / 9));
      end if;
      u1 <= to_unsigned(i mod 16, 4);
      u2 <= to_unsigned(i / 5 mod 16, 4);
      i1 <= i;
      i2 <= -i;
      r1 <= (f => (sl(i), sl(i / 9), sl(i + 1), '0'), g => sl(i), n => i);
      r2 <= (f => (sl(i / 3), 'U', sl(i), '1'), g => sl(i / 9), n => i * 2);
      -- Sampled between the whole nanoseconds at which the outputs change (the order of
      -- reports of two processes in one cycle is not defined).
      wait for 500 ps;
      report integer'image(i) & " " & to_string(m1) & to_string(m2) & to_string(m3) & to_string(m4)
        & to_string(m5) & to_string(m6) & to_string(m7) & to_string(m8) & to_string(m9)
        & to_string(mi) & " " & to_string(mc1) & to_string(mc2) & to_string(mc3) & to_string(mc5)
        & to_string(mc6) & to_string(mcv) & " " & to_string(bm1) & to_string(bm2) & boolean'image(bb1)
        & to_string(bvm);
      report to_string(vm1) & " " & to_string(vm2) & " " & to_string(vm3) & " " & to_string(vm4)
        & " " & to_string(vm5) & " " & to_string(wm1(69 downto 62)) & to_string(wm1(3 downto 0))
        & " " & to_string(wm2(69 downto 62)) & to_string(wm2(3 downto 0)) & " " & to_string(um)
        & " " & integer'image(im) & " " & to_string(rm.f) & to_string(rm.g) & integer'image(rm.n);
      report boolean'image(e1) & boolean'image(e2) & boolean'image(e3) & boolean'image(e4)
        & boolean'image(e5) & boolean'image(e6) & boolean'image(e7) & boolean'image(e8)
        & boolean'image(e9) & boolean'image(e10) & boolean'image(e11) & boolean'image(e12)
        & boolean'image(e13) & to_string(e14) & " " & to_string(g1) & to_string(g2)
        & to_string(g3) & to_string(g5) & boolean'image(g6) & to_string(g8) & " " & to_string(g9)
        & " " & to_string(g4(69 downto 62)) & to_string(g4(3 downto 0)) & " "
        & to_string(g7(69 downto 62)) & to_string(g7(3 downto 0)) & " " & to_string(n1)
        & to_string(n2) & to_string(n3) & to_string(n4) & to_string(n5) & to_string(n6)
        & to_string(n7) & to_string(n8) & to_string(n9);
      wait for 500 ps;
    end loop;
    wait for 10 ns;
    report "done";
    wait;
  end process;

  -- Time and delta of every change of a few outputs.
  watch : process (m1, m6, m7, m8, e1, e7, wm1, rm)
  begin
    report "w " & to_string(m1) & to_string(m6) & to_string(m7) & to_string(m8) & " "
      & boolean'image(e1) & boolean'image(e7) & " " & to_string(wm1(69 downto 62))
      & to_string(wm1(3 downto 0)) & " " & to_string(rm.f) & integer'image(rm.n) & " ev="
      & boolean'image(m1'event) & boolean'image(e1'event) & boolean'image(wm1'event)
      & boolean'image(rm'event);
  end process;
end;
