-- difftest: top=sigrec_whole std=2008
-- A whole record signal read and driven in one go (#677): leaves of every kind (logic scalars,
-- BIT, BOOLEAN, narrow and wide integers, enumerations, real, logic and byte arrays, nested
-- records), a whole drive at once and with a delay, partial drives from two processes, a
-- resolved leaf with two drivers, and a record port (in and out, collapsed onto its actual).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package sigrec_whole_pkg is
  type color is (red, green, blue);
  type ints is array (0 to 2) of integer range -100 to 100;
  type inner is record
    p : std_logic;
    b : bit;
    ok : boolean;
    q : std_logic_vector(3 downto 0);
  end record;
  type rec is record
    a : unsigned(7 downto 0);
    n : integer range -5 to 5;
    big : integer;
    nat : natural range 0 to 255;
    c : color;
    r : real;
    bv : bit_vector(1 to 3);
    iv : ints;
    sl : std_logic;
    wide : std_logic_vector(69 downto 0);
    i1 : inner;
    i2 : inner;
  end record;
  constant zero : rec := (a => x"00", n => 0, big => 0, nat => 0, c => red, r => 0.0, bv => "000",
                          iv => (0, 0, 0), sl => '0', wide => (others => '0'),
                          i1 => ('0', '0', false, "0000"), i2 => ('0', '0', false, "0000"));
  function img (x : rec) return string;
end package;
package body sigrec_whole_pkg is
  function img (x : rec) return string is
  begin
    return to_string(to_integer(x.a)) & "," & integer'image(x.n) & "," & integer'image(x.big) & "," &
      integer'image(x.nat) & "," & color'image(x.c) & "," & integer'image(integer(x.r * 10.0)) & "," &
      to_string(x.bv) & "," & integer'image(x.iv(0)) & integer'image(x.iv(1)) & integer'image(x.iv(2)) & "," &
      std_logic'image(x.sl) & "," & to_hstring(x.wide) & "," &
      std_logic'image(x.i1.p) & bit'image(x.i1.b) & boolean'image(x.i1.ok) & to_string(x.i1.q) & "," &
      std_logic'image(x.i2.p) & bit'image(x.i2.b) & boolean'image(x.i2.ok) & to_string(x.i2.q);
  end;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.sigrec_whole_pkg.all;
entity sigrec_whole_unit is
  port (clk : in std_logic; d : in rec; q : out rec);
end;
architecture a of sigrec_whole_unit is
begin
  process (clk)
    variable v : rec;
  begin
    if rising_edge(clk) then
      v := d;
      v.n := -v.n;
      v.i2.q := not d.i1.q;
      q <= v;
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.sigrec_whole_pkg.all;
entity sigrec_whole is end;
architecture t of sigrec_whole is
  signal clk : std_logic := '0';
  constant seed : rec := (a => x"05", n => 3, big => -7, nat => 250, c => blue, r => 1.5, bv => "101",
          iv => (1, -2, 3), sl => '1', wide => (69 => '1', 3 => '1', others => '0'),
          i1 => ('1', '1', true, "1010"), i2 => ('0', '0', false, "0001"));
  signal r : rec := seed;
  signal rin, late, portq : rec := zero;
  signal part : rec := zero;
  type two is record x : std_logic; y : std_logic_vector(1 downto 0); end record;
  signal tw : two;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  -- Whole read of r, whole drive of rin at once, whole drive of late 3 ns later.
  comb : process (r)
    variable v : rec;
  begin
    v := r;
    v.a := r.a + 1;
    v.n := (r.n + 2) mod 6 - 2;
    v.big := (r.big mod 1000003) * 2000 + 17;
    v.nat := (r.nat + 7) mod 256;
    v.c := color'val((color'pos(r.c) + 1) mod 3);
    v.r := r.r + 0.5;
    v.bv := not r.bv;
    v.iv(1) := (r.iv(1) + 103) mod 201 - 100;
    v.sl := not r.sl;
    v.wide := r.wide(68 downto 0) & r.wide(69);
    v.i1.p := not r.i1.p;
    v.i1.b := not r.i1.b;
    v.i1.ok := not r.i1.ok;
    v.i1.q := std_logic_vector(unsigned(r.i1.q) + 1);
    v.i2 := r.i1;
    rin <= v;
    late <= v after 3 ns;
  end process;

  process (clk) begin
    if rising_edge(clk) then r <= rin; end if;
  end process;

  -- Record port: the instance reads d (whole), drives q (whole).
  u : entity work.sigrec_whole_unit port map (clk => clk, d => r, q => portq);

  -- Two processes drive halves of one record signal (partial drives, no whole drive).
  p1 : process (clk) begin
    if rising_edge(clk) then part.a <= r.a; part.i1 <= r.i1; part.wide <= r.wide; end if;
  end process;
  p2 : process (clk) begin
    if rising_edge(clk) then part.n <= r.n; part.iv <= r.iv; part.i2 <= r.i2; end if;
  end process;

  -- Resolved leaves with two drivers, each driving the whole record.
  ra : process (clk) begin
    if rising_edge(clk) then tw <= (x => '1', y => "Z0"); end if;
  end process;
  rb : process (clk) begin
    if rising_edge(clk) then tw <= (x => 'Z', y => "1Z"); end if;
  end process;

  chk : process
    variable errors : natural := 0;
  begin
    for i in 1 to 40 loop
      wait until rising_edge(clk);
      wait for 1 ns;
      -- late is rin delayed by 3 ns: at 1 ns past the edge both still hold the previous values
      -- or the new ones; the invariant is that late equals the rin of 3 ns ago.
      if i mod 10 = 0 then
        report integer'image(i) & " r=" & img(r);
        report integer'image(i) & " rin=" & img(rin);
        report integer'image(i) & " late=" & img(late);
        report integer'image(i) & " port=" & img(portq);
        report integer'image(i) & " part=" & img(part);
      end if;
    end loop;
    wait for 4 ns;
    assert late = rin report "late /= rin" severity failure;
    assert tw.x = '1' and tw.y = "10" report "resolved leaves wrong: " & std_logic'image(tw.x) & to_string(tw.y) severity failure;
    assert part.a = r.a or part.a = r.a - 1 report "partial drive of a" severity failure;
    report "final tw=" & std_logic'image(tw.x) & to_string(tw.y);
    done <= true;
    wait;
  end process;
end;
