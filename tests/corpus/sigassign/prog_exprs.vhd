-- difftest: top=tb_prog_exprs std=2008
-- Concurrent assignments of expression trees (more than one logical operator, relations
-- inside), compound conditions, chains of three or more arms and constant conditions (#788):
-- values over all nine std_ulogic values, deltas, events, delays per arm and transport, a
-- resolved target with two such drivers, an in port, vectors wider than 64 elements, slices and
-- elements, records, BIT and BOOLEAN, and the nearby shapes that stay processes (numeric_std's
-- "=", a selected assignment, unaffected, a guard, postponed, a rejection limit, a concatenation
-- arm) must be those of the equivalent processes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity prog_inner is
  port (i, j, k, s : in std_ulogic; o, p : out std_ulogic);
end;

architecture a of prog_inner is
begin
  o <= (i and j) or (k and not i);
  p <= i when s = '1' and j = '1' else j when k = '1' else '0';
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_prog_exprs is end;

architecture t of tb_prog_exprs is
  constant K4 : std_logic_vector(3 downto 0) := "1010";
  constant KW : std_logic_vector(69 downto 0) := (others => '1');
  constant G_ON : boolean := true;
  constant G_OFF : boolean := false;
  type state_t is (S_IDLE, S_RUN, S_WAIT, S_DONE, S_FAIL);
  type big_t is (L0, L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16, L17, L18, L19);
  type rs_t is record
    state : state_t;
    v : std_logic;
  end record;
  type rec is record
    f : std_logic_vector(3 downto 0);
    g : std_logic;
    n : integer;
  end record;
  signal a, b, c, d, s, en, tt : std_logic := '0';
  signal v1, v2, v3 : std_logic_vector(3 downto 0) := "0000";
  signal w1, w2, w3 : std_logic_vector(69 downto 0) := (others => '0');
  signal ba, bb, bc, bs : bit := '0';
  signal bv1, bv2 : bit_vector(2 downto 0) := "000";
  signal bo, bl : boolean := false;
  signal u1, u2 : unsigned(3 downto 0) := "0000";
  signal r1, r2, r3 : rec := (f => "0000", g => '0', n => 0);
  -- trees
  signal t1, t2, t3, t4, t5, t6, t7, t8, t9, t10 : std_logic;
  signal tv1, tv2, tv3, tv4 : std_logic_vector(3 downto 0);
  signal tv5 : std_logic_vector(1 downto 0);
  signal tw1, tw2, tw3 : std_logic_vector(69 downto 0);
  signal tb1, tb2 : bit;
  signal tbv : bit_vector(2 downto 0);
  signal tc1, tc2, tc3, tc4, tc5, tc6, tc7, tc8 : boolean;
  signal td1, td2, td3 : std_logic;
  signal tm, tmi, tpi, tq : std_logic;
  -- conditions
  signal c1, c2, c3, c4, c5, c6, c7, c9, c11, c12, c13, c14 : std_logic;
  signal c8 : std_logic_vector(3 downto 0);
  signal c10 : std_logic_vector(69 downto 0);
  -- chains
  signal m1, m2, m4, m7, m9, m10 : std_logic;
  signal m3, m3b : std_logic_vector(3 downto 0);
  signal m5 : rec;
  signal m6 : std_logic_vector(69 downto 0);
  signal m8 : std_logic_vector(3 downto 0);
  signal mb : bit;
  signal mr : std_logic;
  signal st : state_t := S_IDLE;
  signal rs : rs_t := (state => S_IDLE, v => '0');
  signal ii : integer := 0;
  signal q1, q2, q3, q4, q5, q6, q8 : std_logic;
  signal q7 : boolean;
  signal bg : big_t := L0;
  signal ch : character := 'a';
  signal q9, q10 : std_logic;
  -- processes still
  signal n1, n2, n4, n5, n6, n7, n8 : std_logic;
  signal n3 : std_logic_vector(1 downto 0);
begin
  u : entity work.prog_inner port map (i => a, j => b, k => c, s => s, o => tmi, p => tpi);
  t1 <= a and b and c;
  t2 <= (a and b) or c;
  t3 <= not (a xor b);
  t4 <= a and not b;
  t5 <= (a nand b) nor (c xnor d);
  t6 <= (a and '0') or b;
  t7 <= a xor b xor c xor d;
  t8 <= '1' and a and not (b or c);
  t9 <= (v1(1) and v2(2)) or (v3(0) and a);
  t10 <= (a or 'H') and (b or 'L');
  tv1 <= (v1 and v2) or v3;
  tv2 <= not v1 and v2;
  tv3 <= (v1 xor "1010") and v2;
  tv4 <= K4 and v1 and not v2;
  tv5 <= (v1(3 downto 2) and v2(1 downto 0)) or v3(2 downto 1);
  tw1 <= (w1 and w2) or w3;
  tw2 <= not (w1 xor KW);
  tw3 <= (w1 and w2 and not w3) or KW;
  tb1 <= (ba and bb) or bc;
  tb2 <= ba xor bb xor bs;
  tbv <= (bv1 and bv2) xor "101";
  tc1 <= (bo and bl) or not bo;
  tc2 <= (a = b) and (c /= d);
  tc3 <= (v1 = v2) or (w1 = w2);
  tc4 <= not (v1 = "1010");
  tc5 <= (a = '1') and (b = '0');
  tc6 <= (a = '1') xor (b = '1') xor bo;
  tc7 <= (v1(1) = '1') nand (K4 = v2);
  tc8 <= (w1 /= KW) and bl and (ba = '1');
  td1 <= (a and b) or c after 2 ns;
  td2 <= transport (a xor b) and c after 3 ns;
  td3 <= a and b and c after 1 ns;
  tm <= a and b;
  tm <= (c or d) and not s when en = '1' else 'Z';
  tq <= (a or b) and c when s = '1' else 'X';
  -- compound conditions
  c1 <= a when s = '1' and en = '1' else b;
  c2 <= a when s = '1' or en = '1' else b;
  c3 <= a when not (s = '1') else b;
  c4 <= '1' when en = '1' and v1 = "1010" else '0';
  c5 <= a when (s = '1' and en = '1') or (v1 = v2) else b;
  c6 <= a when bo and bl else b;
  c7 <= a when bo and (v1(1) = '1') else b;
  c8 <= v1 when s = '1' and a = '1' else v2;
  c9 <= a when s = '1' xor en = '1' else b;
  c10 <= w1 when w2 = KW or s = '1' else w3;
  c11 <= a when G_ON else b;
  c12 <= a when G_ON and s = '1' else b;
  c13 <= a when not G_ON else b;
  c14 <= a when G_OFF or en = '1' else b after 2 ns;
  -- chains of arms
  m1 <= a when s = '1' else b when en = '1' else c;
  m2 <= a after 1 ns when s = '1' else b after 2 ns when en = '1' else c after 3 ns when tt = '1'
        else d after 4 ns;
  m3 <= v1 when s = '1' else v2 when en = '1' else (others => '0');
  m3b <= "0001" when s = '0' else "0010" when en = '0' else v3;
  m4 <= '1' when s = '0' else '0' when s = '1' else 'X';
  m5 <= r1 when s = '1' else r2 when en = '1' else r3;
  m6 <= w1 when s = '1' else w2 when en = '1' else KW;
  m7 <= (a and b) when s = '1' else (c or d) when en = '1' else not a;
  m8 <= v1 and v2 when s = '1' else v3 or v1 when a = '1' else not v2;
  mb <= ba when bs = '1' else bb when bo else bc;
  m9 <= transport a when s = '1' else b after 2 ns when en = '1' else c;
  m10 <= a when s = '1' and en = '1' else b when s = '1' or tt = '1' else d when a = b else 'Z';
  mr <= a when s = '1' else 'Z';
  mr <= b when s = '0' and en = '1' else c when s = '0' else 'Z';
  -- Tests of enumeration and integer signals.
  q1 <= '1' when st = S_RUN else '0';
  q2 <= a when st = S_RUN and en = '1' else b;
  q3 <= a when (st = S_IDLE) or (st = S_WAIT) else '0';
  q4 <= a when st /= S_IDLE else b when ii = 3 else c;
  q5 <= '1' when ii = -1 else '0';
  q6 <= a when 1000000 = ii else b;
  q7 <= st = S_RUN;
  q8 <= '1' when rs.state = S_WAIT and rs.v = '1' else '0';
  -- Enumeration positions beyond the logic codes, and a character.
  q9 <= '1' when bg = L17 or (bg /= L2 and a = '1') else '0';
  q10 <= a when ch = 'z' else b;
  -- Not kernel processes: still processes.
  n1 <= a when u1 = u2 else b when s = '1' else c;
  n2 <= a when s = '1' else unaffected;
  n3 <= a & b when s = '1' else c & d when en = '1' else (others => '0');
  blk : block (en = '1')
  begin
    n4 <= guarded a when s = '1' else b when tt = '1' else c;
  end block;
  postponed n5 <= (a and b) after 1 ns when s = '1' else c after 1 ns;
  n6 <= reject 1 ns inertial (a and b) after 2 ns when s = '1' else c after 2 ns;
  with s select n7 <= a when '1', b when '0', c when others;
  n8 <= a when (u1 = u2) and s = '1' else b;

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
      c <= sl(i / 3);
      d <= sl(i / 27 + i);
      en <= sl(i / 3 + i / 81);
      tt <= sl(i * 5 / 7);
      bs <= bit'val(i mod 2);
      ba <= bit'val(i / 2 mod 2);
      bb <= bit'val(i / 4 mod 2);
      bc <= bit'val(i / 8 mod 2);
      bo <= i mod 3 = 0;
      bl <= i mod 4 = 0;
      bv1 <= (bit'val(i mod 2), bit'val(i / 2 mod 2), '1');
      bv2 <= (bit'val(i / 4 mod 2), '1', bit'val(i mod 2));
      v1 <= (sl(i), sl(i / 9), sl(i + 4), '1');
      v2 <= (sl(i * 7), 'H', sl(i / 9), sl(i));
      v3 <= (sl(i / 3), sl(i / 27), 'L', sl(i * 2));
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
      w3 <= (others => sl(i / 3));
      w3(69) <= sl(i / 27);
      u1 <= to_unsigned(i mod 16, 4);
      u2 <= to_unsigned(i / 5 mod 16, 4);
      r1 <= (f => (sl(i), sl(i / 9), sl(i + 1), '0'), g => sl(i), n => i);
      r2 <= (f => (sl(i / 3), 'U', sl(i), '1'), g => sl(i / 9), n => i * 2);
      r3 <= (f => (sl(i / 27), 'U', sl(i), '1'), g => sl(i / 3), n => -i);
      st <= state_t'val(i mod 5);
      bg <= big_t'val(i mod 20);
      ch <= character'val(100 + i mod 30);
      rs <= (state => state_t'val(i / 5 mod 5), v => sl(i / 25));
      if i mod 11 = 0 then
        ii <= 1000000;
      else
        ii <= i mod 7 - 3;
      end if;
      wait for 500 ps;
      report integer'image(i) & " " & to_string(t1) & to_string(t2) & to_string(t3) & to_string(t4)
        & to_string(t5) & to_string(t6) & to_string(t7) & to_string(t8) & to_string(t9)
        & to_string(t10) & " " & to_string(tv1) & " " & to_string(tv2) & " " & to_string(tv3)
        & " " & to_string(tv4) & " " & to_string(tv5) & " " & to_string(tw1(69 downto 62))
        & to_string(tw1(3 downto 0)) & " " & to_string(tw2(69 downto 62)) & to_string(tw2(3 downto 0))
        & " " & to_string(tw3(69 downto 62)) & to_string(tw3(3 downto 0)) & " " & to_string(tb1)
        & to_string(tb2) & to_string(tbv);
      report boolean'image(tc1) & boolean'image(tc2) & boolean'image(tc3) & boolean'image(tc4)
        & boolean'image(tc5) & boolean'image(tc6) & boolean'image(tc7) & boolean'image(tc8)
        & " " & to_string(td1) & to_string(td2) & to_string(td3) & " " & to_string(tm)
        & to_string(tmi) & to_string(tpi) & to_string(tq) & " " & to_string(c1) & to_string(c2)
        & to_string(c3) & to_string(c4) & to_string(c5) & to_string(c6) & to_string(c7) & " "
        & to_string(c8) & " " & to_string(c9) & " " & to_string(c10(69 downto 62))
        & to_string(c10(3 downto 0)) & " " & to_string(c11) & to_string(c12) & to_string(c13)
        & to_string(c14);
      report to_string(m1) & to_string(m2) & " " & to_string(m3) & " " & to_string(m3b) & " "
        & to_string(m4) & " " & to_string(m5.f) & to_string(m5.g) & integer'image(m5.n) & " "
        & to_string(m6(69 downto 62)) & to_string(m6(3 downto 0)) & " " & to_string(m7) & " "
        & to_string(m8) & " " & to_string(mb) & " " & to_string(m9) & to_string(m10)
        & to_string(mr) & " " & to_string(n1) & to_string(n2) & to_string(n3) & to_string(n4)
        & to_string(n5) & to_string(n6) & to_string(n7) & to_string(n8) & " " & to_string(q1)
        & to_string(q2) & to_string(q3) & to_string(q4) & to_string(q5) & to_string(q6)
        & boolean'image(q7) & to_string(q8) & to_string(q9) & to_string(q10);
      wait for 500 ps;
    end loop;
    wait for 10 ns;
    report "done";
    wait;
  end process;

  -- Time and delta of every change of a few outputs.
  watch : process (t2, t7, tv1, tw1, tc2, tc3, c1, c5, m1, m2, m5, m7, mr, tm, q1, q4, q7)
  begin
    report "w " & to_string(t2) & to_string(t7) & to_string(tv1) & " " & to_string(tw1(69 downto 62))
      & " " & boolean'image(tc2) & boolean'image(tc3) & " " & to_string(c1) & to_string(c5) & " "
      & to_string(m1) & to_string(m2) & to_string(m7) & to_string(mr) & to_string(tm) & " "
      & to_string(m5.f) & integer'image(m5.n) & " " & to_string(q1) & to_string(q4) & boolean'image(q7)
      & " ev=" & boolean'image(t2'event)
      & boolean'image(tv1'event) & boolean'image(tc2'event) & boolean'image(c1'event)
      & boolean'image(m1'event) & boolean'image(m2'event) & boolean'image(m5'event)
      & boolean'image(tm'event) & boolean'image(q1'event) & boolean'image(q7'event);
  end process;
end;
