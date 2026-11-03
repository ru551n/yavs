-- difftest: top=held_record std=2008 ignore=order
-- Whole record drives of the value a signal holds, around what must see them: force, release,
-- an entry or a waveform of the same run, parts, resolved records, delays, ports, the deltas of a
-- run that only holds. The kernel may compare such a drive with an image of the record and skip it.
-- ignore=order: reports of different processes in one cycle.
library ieee; use ieee.std_logic_1164.all;
package held_record_p is
  type color is (red, green, blue);
  type ints is array (0 to 2) of integer;
  type rec is record
    a : std_logic;
    b : std_logic_vector(7 downto 0);
    c : std_logic;
    i : integer;
    e : color;
    w : std_logic_vector(69 downto 0);
    n : std_logic_vector(3 to 2);
    x : ints;
    t : time;
    f : boolean;
    y : std_logic_vector(129 downto 0);
  end record;
  constant init : rec := ('0', x"00", '0', -1, red, (others => '0'), "", (1, -2, 3), 1 ns, false,
                          (others => '0'));
  type recs is array (natural range <>) of rec;
  type pair is record
    a : std_logic;
    b : std_logic_vector(3 downto 0);
  end record;
  type pairs is array (natural range <>) of pair;
  function join (p : pairs) return pair;
  subtype joined is join pair;
  function image (r : rec) return string;
  function image (p : pair) return string;
end package;

package body held_record_p is
  function join (p : pairs) return pair is
    variable r : pair := ('0', "0000");
  begin
    for i in p'range loop
      r.a := r.a or p(i).a;
      r.b := r.b or p(i).b;
    end loop;
    return r;
  end function;
  function image (r : rec) return string is
  begin
    return std_logic'image(r.a) & to_string(r.b) & std_logic'image(r.c) & " " & integer'image(r.i)
      & " " & color'image(r.e) & " " & to_hstring(r.w) & " " & integer'image(r.x(0)) & " "
      & integer'image(r.x(1)) & " " & integer'image(r.x(2)) & " " & time'image(r.t) & " "
      & boolean'image(r.f) & " " & to_hstring(r.y);
  end function;
  function image (p : pair) return string is
  begin
    return std_logic'image(p.a) & to_string(p.b);
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.held_record_p.all;
entity held_record_port is
  generic (g : natural);
  port (clk : in std_logic; n : in natural; o : out rec);
end;
architecture a of held_record_port is
begin
  process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 30 then v.b := x"0f"; end if;
      o <= v;
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all;
use work.held_record_p.all;
entity held_record_u is
  generic (g : natural);
end;
architecture a of held_record_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal changes, forced, entries, waved, delayed, seen, oport, only : rec := init;
  signal parts : recs(0 to 3) := (others => init);
  signal shared_a, shared_b, fields : pair := ('0', "0000");
  signal flag : std_logic := '0';
  signal nibble : std_logic_vector(3 downto 0) := x"0";
  signal res1 : pair := ('Z', "ZZZZ");
  signal swap : pair := ('Z', "ZZZZ");
  signal res2 : joined;
  signal slot1, slot2 : rec := init;
  signal clk2 : std_logic := '0';
  signal held_late : rec := init;
  procedure put (signal s : out rec; v : rec) is
  begin
    s <= v;
  end procedure;
begin
  clk <= not clk after 5 ns when n < 110 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;

  -- One leaf at a time changes under the held rest: scalars, a negative integer, one plane of an
  -- element, the last and first element of wide leaves, an element of an integer array.
  changes_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      case n is
        when 30 => v.a := '1';
        when 32 => v.b(0) := '1';
        when 34 => v.b(7) := 'X';
        when 36 => v.b(7) := '0';
        when 38 => v.c := 'H';
        when 40 => v.i := -2;
        when 42 => v.i := 2147483647;
        when 44 => v.e := blue;
        when 46 => v.w(69) := '1';
        when 48 => v.w(0) := 'Z';
        when 50 => v.x(2) := -3;
        when 52 => v.t := 2 ns;
        when 54 => v.f := true;
        when 56 => v.y(129) := '1';
        when 58 => v.y(64) := '1';
        when 60 => v.y(0) := 'U';
        when 62 => v := init;
        when others => null;
      end case;
      changes <= v;
    end if;
  end process;

  -- Force and release under held drives: the driving value differs from the forced one, then the
  -- value driven is the one forced, so only the driving value tells it from a skip.
  forced_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 34 then v.b := x"5a"; v.a := '1'; v.i := 77; end if;
      if n = 44 then v.b := x"33"; end if;
      if n = 54 then v := init; end if;
      forced <= v;
    end if;
  end process;
  forcer : process
  begin
    wait until n = 30;
    forced.a <= force '1';
    forced.b <= force x"5a";
    forced.i <= force 77;
    wait until n = 38;
    forced.a <= release;
    forced.b <= release;
    forced.i <= release;
    wait until n = 42;
    forced <= force ('1', x"33", '1', 5, green, (others => '1'), "", (0, 0, 0), 3 ns, true,
                     (others => '1'));
    wait until n = 48;
    forced <= release;
    wait until n = 52;
    forced.w <= force (others => 'X');
    wait for 0 ns;
    forced.w <= release;
    wait;
  end process;

  -- Entries of the same run, before and after the held value; a kept field before the record.
  entries_p : process (clk)
    variable v : rec := init;
    variable w : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      case n is
        when 30 => entries.b <= x"aa"; entries <= v;
        when 32 => entries.a <= '1'; entries <= v;
        when 34 => entries.i <= 9; entries <= v;
        when 36 => w.b := x"11"; entries <= w; entries <= v;
        when 38 => entries <= v; entries.b <= x"cc";
        when 40 => entries <= v;
        when 42 => entries.a <= '1'; entries.c <= '1'; entries <= v; entries.c <= '1';
        when 44 => entries <= v;
        when 46 => entries.a <= entries.a; entries <= v; entries.a <= '1';
        when 48 => entries.w(3 downto 0) <= x"f"; entries <= v;
        when others => entries <= v;
      end case;
    end if;
  end process;

  -- A waveform on the driver: the zero-delay drive of what is held takes it away.
  waved_p : process (clk)
    variable v : rec := init;
    variable w : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      w := init; w.b := x"77"; w.a := '1';
      case n is
        when 30 => waved <= w after 25 ns;
        when 40 => waved.a <= '1' after 25 ns;
        when 44 => null;
        when 50 => waved <= transport w after 25 ns;
        when 60 => waved <= w after 5 ns, v after 15 ns;
        when 63 => null;
        when 70 => waved <= w after 25 ns;
        when 71 => waved <= v after 3 ns;
        when others => waved <= v;
      end case;
    end if;
  end process;

  -- Delays on the value held, with and without a limit, and a change pending under them.
  delayed_p : process (clk)
    variable v : rec := init;
    variable w : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      w := init; w.b := x"99"; w.c := '1';
      case n is
        when 30 => delayed <= transport w after 22 ns;
        when 31 => delayed <= transport v after 3 ns;
        when 40 => delayed <= w after 22 ns;
        when 41 => delayed <= reject 1 ns inertial v after 3 ns;
        when 50 => delayed <= w after 2 ns;
        when others => delayed <= v after 2 ns;
      end case;
    end if;
  end process;

  -- Whole, an element, a slice and a run-time element of an array of records.
  parts_p : process (clk)
    variable v : rec := init;
    variable s : recs(0 to 3) := (others => init);
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 30 then s(1).b := x"21"; end if;
      if n = 40 then s(2).i := 4; end if;
      if n = 50 then s(3).a := '1'; end if;
      if n mod 3 = 0 then
        parts <= s;
      elsif n mod 3 = 1 then
        parts(1) <= s(1);
        parts(2 to 3) <= s(2 to 3);
      else
        parts(n mod 4) <= s(n mod 4);
      end if;
    end if;
  end process;

  -- Two sources of a record, plain and with a resolution function of the record.
  res_a : process (clk)
    variable v : pair := ('1', "ZZ01");
    variable w : pair := ('0', "0001");
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 40 then v.b := "ZZ11"; w.b := "0011"; end if;
      res1 <= v;
      res2 <= w;
      -- The sources swap under a value that stays; what each holds shows when the other goes.
      if n < 40 then swap <= ('1', "1010"); else swap <= ('Z', "ZZZZ"); end if;
    end if;
  end process;
  res_b : process (clk)
    variable v : pair := ('Z', "01ZZ");
    variable w : pair := ('0', "0100");
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 50 then v.b := "11ZZ"; w.b := "1100"; end if;
      res1 <= v;
      res2 <= w;
      if n >= 40 and n < 50 then swap <= ('1', "1010"); else swap <= ('Z', "ZZZZ"); end if;
    end if;
  end process;

  -- A record read and driven by several, so its image has several users.
  shared_w : process (clk)
    variable v : pair := ('0', "0000");
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 16 = 15 then v.b(0) := not v.b(0); end if;
      shared_a <= v;
      shared_b <= shared_a;
    end if;
  end process;

  -- Single drives of the same run around the whole record: held, changed, and of other signals,
  -- which compiled code compares itself or keeps for later.
  fields_p : process (clk)
    variable v : pair := ('0', "0000");
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      flag <= flag;
      nibble <= nibble;
      fields.a <= fields.a;
      fields <= v;
      fields.a <= v.a;
      if n = 30 then
        flag <= '1';
        fields.b <= "1010";
        fields <= v;
      elsif n = 32 then
        fields <= v;
        fields.a <= '1';
      elsif n = 34 then
        v.a := '1';
        fields.a <= '1';
        fields <= v;
      elsif n = 36 then
        nibble <= x"6";
        fields <= v;
      end if;
    end if;
  end process;

  -- Two signals through one drive of a procedure.
  slots_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n = 30 then v.b := x"31"; end if;
      if n mod 2 = 0 then put(slot1, v); end if;
      if n = 35 then v.b := x"32"; end if;
      if n mod 2 = 1 then put(slot2, v); end if;
    end if;
  end process;

  u : entity work.held_record_port generic map (g) port map (clk, n, oport);

  -- A run that only holds keeps its delta, which the postponed process names.
  only_p : process (clk)
    variable v : rec := init;
  begin
    if falling_edge(clk) then
      if g > 1000 or n < 0 then report "g"; end if;
      only <= v;
    end if;
  end process;
  postponed process (clk)
  begin
    if clk = '0' then
      report "falling " & integer'image(n);
    end if;
  end postponed process;

  -- Read at the clock: the references differ on whether a release of the value held is an event.
  -- A delay on the value held, with nothing else in the cycle, makes no delta of this time.
  clk2 <= not clk2 after 7 ns when n < 40 else '0';
  late_p : process (clk2)
    variable v : rec := init;
  begin
    if falling_edge(clk2) then
      if g > 1000 or n < 0 then report "g"; end if;
      held_late <= v after 2 ns;
    end if;
  end process;
  postponed process (clk2)
  begin
    if clk2 = '0' then
      report "late " & integer'image(n);
    end if;
  end postponed process;

  watch_forced : process (clk)
  begin
    if falling_edge(clk) then
      report image(forced);
    end if;
  end process;

  watch : process (changes, entries, waved, delayed, parts, res1, res2, swap, shared_a, shared_b,
                   fields, flag, nibble, slot1, slot2, oport)
  begin
    report image(changes);
    report image(entries);
    report image(waved);
    report image(delayed);
    report image(parts(0)) & " " & image(parts(1));
    report image(parts(2)) & " " & image(parts(3));
    report image(res1) & " " & image(res2) & " " & image(swap);
    report image(shared_a) & " " & image(shared_b);
    report image(fields) & " " & std_logic'image(flag) & " " & to_string(nibble);
    report image(slot1);
    report image(slot2);
    report image(oport);
  end process;
end;

entity held_record is
end;
architecture a of held_record is
begin
  u0 : entity work.held_record_u generic map (0);
  u1 : entity work.held_record_u generic map (1);
end;
