-- Subprogram calls in cycle mode against the normal mode on 0/1 inputs, for a width that the test
-- fills in: functions and procedures of a package and of a process, inlined. Unconstrained formals
-- given `to` objects and slices, nested calls, early returns in loops, a recursion whose depth the
-- lengths fix, record results, `out` and `inout` parameters (by reference, as the normal mode
-- passes them), signal parameters, a concurrent procedure call, `exit` and `next`, aliases.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_calls_pkg is
  type ends_t is record
    lo, hi : std_ulogic;
    n : natural range 0 to 255;
  end record;
  function or_red (d : std_ulogic_vector) return std_ulogic;
  function rev (d : std_ulogic_vector) return std_ulogic_vector;
  function rep (d : std_ulogic; n : natural) return std_ulogic_vector;
  function to_sl (b : boolean) return std_ulogic;
  function first_one (d : std_ulogic_vector) return integer;
  function safe_div (a, b : integer) return integer;
  function pop (d : std_ulogic_vector) return natural;
  function ends (d : std_ulogic_vector) return ends_t;
  function exit_or (d : std_ulogic_vector) return integer;
  function tri (n : natural) return natural;
  procedure split (d : in std_ulogic_vector; signal lo_s : out std_ulogic; n : out natural;
                   acc : inout unsigned);
  procedure bump (x : out natural);
  procedure add2 (x : inout unsigned; y : in unsigned);
  procedure halves (d : in std_ulogic_vector; signal l, h : out std_ulogic);
end package;

package body cycle_calls_pkg is
  function or_red (d : std_ulogic_vector) return std_ulogic is
    variable v : std_ulogic;
  begin
    v := '0';
    for i in d'range loop
      v := v or d(i);
    end loop;
    return v;
  end function;

  function rev (d : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(d'length - 1 downto 0);
  begin
    for i in 0 to d'length - 1 loop
      r(d'length - 1 - i) := d(d'low + i);
    end loop;
    return r;
  end function;

  function rep (d : std_ulogic; n : natural) return std_ulogic_vector is
    variable v : std_ulogic_vector(n - 1 downto 0);
  begin
    v := (others => d);
    return v;
  end function;

  function to_sl (b : boolean) return std_ulogic is
  begin
    if b then
      return '1';
    else
      return '0';
    end if;
  end function;

  -- The index of the leftmost '1', -1 for none: an early return inside a loop.
  function first_one (d : std_ulogic_vector) return integer is
  begin
    for i in d'range loop
      if d(i) = '1' then
        return i;
      end if;
    end loop;
    return -1;
  end function;

  -- The division after the early return must not run (and fail) when `b` is 0.
  function safe_div (a, b : integer) return integer is
  begin
    if b = 0 then
      return 0;
    end if;
    return a / b;
  end function;

  -- A recursion whose depth the lengths fix.
  function pop (d : std_ulogic_vector) return natural is
    variable x : std_ulogic_vector(d'length - 1 downto 0);
  begin
    x := d;
    if d'length = 1 then
      if x(0) = '1' then
        return 1;
      else
        return 0;
      end if;
    end if;
    return pop(x(d'length - 1 downto d'length / 2)) + pop(x(d'length / 2 - 1 downto 0));
  end function;

  function ends (d : std_ulogic_vector) return ends_t is
  begin
    return (lo => d(d'right), hi => d(d'left), n => d'length mod 256);
  end function;

  -- A `return` after an `exit` that may have been taken: the `return` after the loop is reached.
  function exit_or (d : std_ulogic_vector) return integer is
  begin
    for i in d'range loop
      exit when d(i) = '1';
      return 5;
    end loop;
    return 7;
  end function;

  -- 0 + 1 + ... + n, run during the lowering where its argument is a constant.
  function tri (n : natural) return natural is
    variable c : natural := 0;
  begin
    for i in 0 to n loop
      c := c + i;
    end loop;
    return c;
  end function;

  -- An early `return` leaves `n` unchanged: `out` scalars are passed by reference.
  procedure split (d : in std_ulogic_vector; signal lo_s : out std_ulogic; n : out natural;
                   acc : inout unsigned) is
  begin
    lo_s <= d(d'right);
    if d(d'left) = '0' then
      return;
    end if;
    n := (first_one(d) + 7) mod 7;
    acc := acc + 1;
  end procedure;

  -- An `out` read before it is written: its `'left`.
  procedure bump (x : out natural) is
  begin
    x := x + 5;
  end procedure;

  procedure add2 (x : inout unsigned; y : in unsigned) is
  begin
    x := x + y;
    x := x + y;
  end procedure;

  procedure halves (d : in std_ulogic_vector; signal l, h : out std_ulogic) is
  begin
    l <= or_red(d);
    h <= d(d'left) xor d(d'right);
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_calls_pkg.all;
entity cycle_calls is
  generic (gw : positive := W);
end entity;
architecture a of cycle_calls is
  signal clk : std_ulogic := '0';
  signal a, b : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal t : std_ulogic_vector(0 to W - 1) := (others => '0');
  signal k : integer range -4 to 4 := 0;
  signal r1, r2, r3, r4, z, zz : std_ulogic_vector(W - 1 downto 0);
  signal zg : std_ulogic_vector(W / 2 downto 0);
  signal r6 : std_ulogic_vector(W downto 0);
  alias bt : std_ulogic_vector(0 to W - 1) is b;
  alias b0 : std_ulogic is b(0);
  signal red : std_ulogic_vector(4 downto 0);
  signal pos, post, dv, cnt, enc, xo : integer range -1000 to 1000;
  signal ep : ends_t;
  signal o_lo, cp_l, cp_h : std_ulogic;
  signal o_n, o_b : natural range 0 to 255;
  signal o_acc : std_ulogic_vector(7 downto 0);
begin
  f1 : process (all)
  begin
    r1 <= rev(a) xor rev(b);
    r2 <= rev(rev(a));
    r3 <= rev(t);
    r4 <= rev(b(W - 1 downto 0));
    red <= or_red(a) & or_red(t) & to_sl(a = b) & or_red(b(W - 1 downto W / 2)) & to_sl(k > 0);
    pos <= first_one(a);
    post <= first_one(t);
    dv <= safe_div(k * 7, k);
    cnt <= pop(a);
    ep <= ends(t);
    xo <= exit_or(a);
    r6 <= rev(bt) & to_sl(b0 = '1');
  end process;

  -- A call in an unrolled loop with the loop parameter as its argument, and a static `exit`.
  loops : process (all)
    variable v : std_ulogic_vector(W - 1 downto 0);
  begin
    v := (others => '0');
    for i in 0 to W - 1 loop
      exit when i = 3;
      v(i downto 0) := rep(a(i), i + 1) xor v(i downto 0);
    end loop;
    z <= v;
  end process;

  -- A loop bound that a pure function of constants gives; a slice whose bounds are attributes of
  -- objects in arithmetic.
  pf : process (all)
    variable s : std_ulogic_vector(W - 1 downto 0);
    variable g : std_ulogic_vector(gw / 2 downto 0);
  begin
    g := a(a'high downto a'length - g'length);
    zg <= g;
    s := (others => '0');
    for i in 0 to minimum(W - 1, tri(2)) loop
      s(i) := a(W - 1 - i);
    end loop;
    zz <= s;
  end process;

  -- `next` and `exit` chosen at run time.
  pri : process (all)
    variable idx : integer range -1 to W - 1;
  begin
    idx := -1;
    for i in 0 to W - 1 loop
      next when b(i) = '0';
      idx := i;
      exit when a(i) = '1';
    end loop;
    enc <= idx;
  end process;

  seq : process (clk)
    variable acc : unsigned(7 downto 0) := x"10";
    variable n, m : natural range 0 to 255;
    procedure tick is
    begin
      acc := acc + 3;
    end procedure;
  begin
    if rising_edge(clk) then
      n := 9;
      split(a, o_lo, n, acc);
      tick;
      bump(m);
      add2(acc, acc);
      o_n <= n;
      o_b <= m;
      o_acc <= std_ulogic_vector(acc);
    end if;
  end process;

  halves(t, cp_l, cp_h);

  main : process
    variable x : unsigned(63 downto 0) := x"9E3779B97F4A7C15";
    variable va, vb : std_ulogic_vector(W - 1 downto 0);
  begin
    for c in 1 to 16 loop
      for j in 0 to W - 1 loop
        va(j) := x(j mod 64) xor x((j * 7 + 3) mod 64);
        vb(j) := x((j * 5 + 1) mod 64);
      end loop;
      if c mod 4 = 0 then vb := va; end if;
      if c mod 5 = 0 then vb := (others => '0'); end if;
      if c mod 7 = 0 then va := (others => '0'); end if;
      a <= va;
      b <= vb;
      t <= vb;
      k <= (c mod 9) - 4;
      x := (x(62 downto 0) & '0') xor (x(63 downto 0) and x"000000000000001B" and (63 downto 0 => x(63)));
      x := x + 16#5851F42D#;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " r1=" & to_hstring(r1) & " r2=" & to_hstring(r2)
        & " r3=" & to_hstring(r3) & " r4=" & to_hstring(r4) & " z=" & to_hstring(z)
        & " zz=" & to_hstring(zz) & " zg=" & to_string(zg) & " r6=" & to_hstring(r6)
        & " red=" & to_string(red) & " pos=" & integer'image(pos) & " post=" & integer'image(post)
        & " dv=" & integer'image(dv) & " cnt=" & integer'image(cnt) & " enc=" & integer'image(enc)
        & " xo=" & integer'image(xo)
        & " ep=" & std_ulogic'image(ep.lo) & std_ulogic'image(ep.hi) & integer'image(ep.n)
        & " lo=" & std_ulogic'image(o_lo) & " n=" & integer'image(o_n) & " m=" & integer'image(o_b)
        & " acc=" & to_hstring(o_acc) & " cp=" & std_ulogic'image(cp_l) & std_ulogic'image(cp_h);
    end loop;
    wait;
  end process;
end architecture;
