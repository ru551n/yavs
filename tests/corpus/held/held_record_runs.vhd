-- difftest: top=held_record_runs std=2008 ignore=order
-- Whole record drives where some fields change and the rest hold: metavalue fields, a nested
-- record, arrays with null and descending ranges, a record with an unconstrained field, and
-- record equality against a record of another subtype. The kernel may compare such a drive with
-- the record's image run by run and drive only what changed.
-- ignore=order: reports of different processes in one cycle.
library ieee; use ieee.std_logic_1164.all;
package held_record_runs_p is
  type color is (red, green, blue);
  type inner is record
    x : std_ulogic;
    y : std_ulogic_vector(0 to 3);
  end record;
  type rec is record
    m : std_ulogic;
    b : std_ulogic_vector(7 downto 0);
    s : inner;
    i : integer;
    e : color;
    f : boolean;
  end record;
  constant init : rec := ('U', x"00", ('0', "0000"), 0, red, false);
  type rich is record
    d : integer_vector(3 downto 0);
    n : std_ulogic_vector(0 to -1);
    z : std_ulogic_vector(1 downto 2);
    w : std_ulogic_vector(69 downto 0);
    t : std_ulogic;
  end record;
  constant rinit : rich := ((others => 0), "", "", (others => '0'), '0');
  type vrec is record
    v : std_ulogic_vector;
    k : integer;
  end record;
  subtype vdown is vrec(v(3 downto 0));
  subtype vup is vrec(v(0 to 3));
  subtype vwide is vrec(v(7 downto 0));
  function image (r : rec) return string;
  function image (r : rich) return string;
  function meta (k : natural) return std_ulogic;
end package;

package body held_record_runs_p is
  function image (r : rec) return string is
  begin
    return std_ulogic'image(r.m) & " " & to_string(r.b) & " " & std_ulogic'image(r.s.x)
      & to_string(r.s.y) & " " & integer'image(r.i) & " " & color'image(r.e) & " "
      & boolean'image(r.f);
  end function;
  function image (r : rich) return string is
  begin
    return integer'image(r.d(3)) & "," & integer'image(r.d(0)) & " " & integer'image(r.n'length)
      & integer'image(r.z'length) & " " & to_hstring(r.w) & " " & std_ulogic'image(r.t);
  end function;
  function meta (k : natural) return std_ulogic is
    constant all9 : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
  begin
    return all9(k mod 9);
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.held_record_runs_p.all;
entity held_record_runs_inst is
  generic (seed : natural);
  port (clk : in std_ulogic; n : in natural; eqs : out natural);
end entity;

architecture a of held_record_runs_inst is
  signal r : rec := init;
  signal rr : rich := rinit;
  signal q : vdown := (v => "0000", k => 0);
  signal c1, c2, c3 : natural := 0;
begin
  eqs <= c1 + c2 + c3;

  -- Leaves the kernel's fast paths take, so compiled code compares the drive with the image.
  lean : process (clk)
    variable v : rec := init;
    variable c : natural := 0;
    -- A call between drives: the optimiser keeps both.
    procedure tick is begin c := c + 1; end procedure;
  begin
    if rising_edge(clk) then
      -- The counter changes at every clock, the rest only now and then: most drives hold in
      -- most runs and change in one or two.
      v.i := (n * seed) mod 5;
      if n mod 7 = seed mod 7 then v.m := meta(n / 7 + seed); end if;
      if n mod 11 = 3 then v.b(n mod 8) := meta(n); end if;
      if n mod 13 = 5 then v.s.x := not v.s.x; v.s.y(n mod 4) := meta(n + 4); end if;
      if n mod 17 = 2 then v.e := color'val((n / 17) mod 3); end if;
      if n mod 29 = 8 then v.f := not v.f; end if;
      if n mod 31 = 30 then v := init; end if;
      r <= v;
      tick;
      -- The same value again in the run: all of it holds.
      if n mod 3 = seed mod 3 then r <= v; end if;
      if v = r then c := c + 1000; end if;
      if v.s = r.s then c := c + 10000; end if;
      c1 <= c;
    end if;
  end process;

  -- Arrays of several integers, null and descending ranges, a vector over 64 elements.
  wide : process (clk)
    variable v : rich := rinit;
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      if n mod 19 = 9 then v.d(n mod 4) := n; end if;
      if n mod 23 = 1 then v.w(n mod 70) := not v.w(n mod 70); end if;
      v.t := meta(n / 5 + seed);
      rr <= v;
      if v = rr then c := c + 1; end if;
      if v.d = rr.d and v.n = rr.n and v.z = rr.z then c := c + 10; end if;
      c2 <= c;
    end if;
  end process;

  -- A record with an unconstrained field, and equality with records of other subtypes.
  sub : process (clk)
    variable u : vup := (v => "0000", k => 0);
    variable g : vwide := (v => x"00", k => 0);
    variable qv : vdown := (v => "0000", k => 0);
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      u.v := (others => meta(n / 3 + seed));
      u.v(n mod 4) := meta(n);
      u.k := n mod 4;
      qv.v := u.v;
      qv.k := u.k;
      q <= qv;
      -- Same length and the other direction, then another length.
      g.v(3 downto 0) := u.v;
      g.k := u.k;
      if qv = q then c := c + 1; end if;
      if u = q then c := c + 10; end if;
      if g /= q then c := c + 100; end if;
      c3 <= c;
    end if;
  end process;

  watch : process (r)
  begin
    report "r" & integer'image(seed) & " " & image(r);
  end process;

  watch_rr : process (rr)
  begin
    report "rr" & integer'image(seed) & " " & image(rr);
  end process;

  watch_q : process (q)
  begin
    report "q" & integer'image(seed) & " " & to_string(q.v) & " " & integer'image(q.k);
  end process;
end architecture;

library ieee; use ieee.std_logic_1164.all;
entity held_record_runs is end entity;

architecture a of held_record_runs is
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  signal e1, e2 : natural := 0;
begin
  clk <= not clk after 5 ns when n < 200 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  i1 : entity work.held_record_runs_inst generic map (seed => 1) port map (clk, n, e1);
  i2 : entity work.held_record_runs_inst generic map (seed => 4) port map (clk, n, e2);

  process
  begin
    wait until n = 200;
    wait for 20 ns;
    report "done " & integer'image(e1) & " " & integer'image(e2);
    wait;
  end process;
end architecture;
