-- CPU idioms against the normal mode (`W` replaced by a width): `'pos` and `'val` of an
-- enumeration and of logic types, `numeric_std`'s `+` and `-` of a vector and a `std_ulogic`, an
-- aggregate as an operand of `&`, and a function on an unconstrained vector given the result of
-- `resize` (whose index range `numeric_std` fixes), `=` and `/=` of records, a slice of a function's
-- result, `'high` of a subtype whose range depends on a generic as a slice bound.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_idioms_pkg is
  type op_t is (op_nop, op_add, op_sub, op_and, op_or, op_xor, op_ld, op_st, op_br);
  function low_ones(v : std_ulogic_vector; n : natural) return natural;
  function swap(v : std_ulogic_vector(7 downto 0)) return std_ulogic_vector;
  type rec_t is record
    a : unsigned(3 downto 0);
    n : natural range 0 to 9;
    e : op_t;
  end record;
end package;
package body cycle_idioms_pkg is
  -- The number of ones among the `n` lowest elements of `v` (`v'right` up).
  function low_ones(v : std_ulogic_vector; n : natural) return natural is
    variable c : natural := 0;
  begin
    for k in 0 to n - 1 loop
      if v(v'right + k) = '1' then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;
  function swap(v : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
    variable r : std_ulogic_vector(7 downto 0);
  begin
    r := v(3 downto 0) & v(7 downto 4);
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_idioms_pkg.all;
entity cycle_idioms_u is
  generic (G : positive);
  port (clk : in std_ulogic; x : in unsigned(W + 7 downto 0); b : in std_ulogic;
        sel : in natural range 0 to 8; lv : in natural range 2 to 3; wide_sel : in natural;
        ra, rb : in rec_t; neq : out std_ulogic; sw : out std_ulogic_vector(3 downto 0);
        ag : out std_ulogic_vector(15 downto 0); eqa : out std_ulogic;
        p : out unsigned(7 downto 0); s : out unsigned(W + 7 downto 0);
        d : out unsigned(W + 7 downto 0); sg : out signed(W + 7 downto 0);
        y : out std_ulogic_vector(W + 7 downto 0); n : out natural; bp : out natural;
        op : out op_t; bv : out std_ulogic; hs : out unsigned(G - 1 downto 0));
end entity;
architecture a of cycle_idioms_u is
  -- A pipeline of one stage: its shift is a null slice given a null slice.
  constant L : positive := 1;
  signal pipe : std_ulogic_vector(1 to L) := (others => '0');
  subtype g_t is unsigned(2 * G - 1 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      p <= to_unsigned(op_t'pos(op_t'val(sel)), 8);
      s <= x + b;
      d <= b - x;
      sg <= signed(x) + b;
      y <= std_ulogic_vector(x(W + 7 downto 3)) & (2 downto 0 => b);
      n <= low_ones(std_ulogic_vector(resize(signed(x(W + 3 downto 0)), W + 8)), W + 8);
      bp <= std_ulogic'pos(b) + boolean'pos(x(0) = '1') + bit'pos(to_bit(x(1)));
      bv <= std_ulogic'val(lv) xor pipe(L);
      pipe(1) <= b;
      pipe(2 to L) <= pipe(1 to L - 1);
      -- A position the type cannot check statically: checked at run time (in range here).
      op <= op_t'val(wide_sel);
      neq <= '1' when ra /= rb else '0';
      sw <= swap(std_ulogic_vector(x(7 downto 0)))(5 downto 2);
      -- Named aggregates as operands: their own range, 0 to 7, element 0 leftmost.
      ag <= std_ulogic_vector(x(7 downto 0)) & (0 => b, 1 to 3 => x(1), 4 to 7 => '0');
      eqa <= '1' when std_ulogic_vector(x(3 downto 0)) = (0 => '1', 1 to 3 => '0') else '0';
      hs <= x(g_t'high downto g_t'low + G);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_idioms_pkg.all;
entity cycle_idioms is
end entity;
architecture t of cycle_idioms is
  signal clk, b, bv : std_ulogic := '0';
  signal x, s, d : unsigned(W + 7 downto 0) := (others => '0');
  signal sg : signed(W + 7 downto 0);
  signal y : std_ulogic_vector(W + 7 downto 0);
  signal sel : natural range 0 to 8 := 0;
  signal lv : natural range 2 to 3 := 2;
  signal wide_sel : natural := 0;
  signal ra, rb : rec_t := ((others => '0'), 0, op_nop);
  signal neq : std_ulogic;
  signal sw : std_ulogic_vector(3 downto 0);
  signal ag : std_ulogic_vector(15 downto 0);
  signal eqa : std_ulogic;
  signal p : unsigned(7 downto 0);
  signal n, bp : natural;
  signal op : op_t;
  signal hs : unsigned(3 downto 0);
begin
  u : entity work.cycle_idioms_u
    generic map (G => 4)
    port map (clk => clk, x => x, b => b, sel => sel, lv => lv, wide_sel => wide_sel, ra => ra, rb => rb,
              neq => neq, sw => sw, ag => ag, eqa => eqa, p => p, s => s, d => d, sg => sg, y => y,
              n => n, bp => bp, op => op, bv => bv, hs => hs);
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 30 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      x <= x(W + 6 downto 0) & r(0);
      if k mod 5 = 0 then
        x <= (others => '1');
      elsif k mod 7 = 0 then
        x <= (W + 7 downto 4 => '0', 3 => '1', 2 downto 0 => '0');
      end if;
      b <= r(3);
      sel <= (sel + 4) mod 9;
      lv <= 2 + to_integer(r(2 downto 2));
      wide_sel <= (k * 7) mod 9;
      ra.a <= r(3 downto 0);
      ra.n <= k mod 10;
      if k mod 3 = 0 then
        rb <= ra;
      end if;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " op=" & op_t'image(op) & " p=" & to_hstring(p)
        & " s=" & to_hstring(s) & " d=" & to_hstring(d) & " sg=" & to_hstring(sg)
        & " y=" & to_hstring(y) & " n=" & integer'image(n) & " bp=" & integer'image(bp)
        & " bv=" & std_ulogic'image(bv) & " neq=" & std_ulogic'image(neq) & " sw=" & to_hstring(sw)
        & " ag=" & to_string(ag) & " eqa=" & std_ulogic'image(eqa) & " hs=" & to_hstring(hs);
    end loop;
    wait;
  end process;
end architecture;
