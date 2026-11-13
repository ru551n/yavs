-- difftest: top=array_order std=2008
-- Predefined "<", "<=", ">", ">=" on one-dimensional arrays (LRM 9.2.3): lexicographic order
-- against a plain element loop, for std_ulogic_vector (every pair of values at the first
-- difference), bit_vector, boolean_vector, string (characters above 127), integer_vector
-- (negative values), an integer type wider than 32 bits and an enumeration; first difference at
-- the left, in the middle, at the right and on both sides of 64-element block boundaries; equal
-- arrays, prefixes, longer and shorter non-prefixes, null arrays, both directions, slices at odd
-- offsets; widths 1, 64, 65, 128, 129, 1024, 1025. Two null arrays are not compared: sim1 orders
-- "" < "" as TRUE (the LRM: a null array is less only than a non-null one).
library ieee;
use ieee.std_logic_1164.all;
entity array_order is end entity;
architecture a of array_order is
  type big is range -1099511627776 to 1099511627776;
  type big_vector is array (natural range <>) of big;
  type e4 is (ea, eb, ec, ed);
  type e4_vector is array (natural range <>) of e4;
  type widths_t is array (natural range <>) of natural;
  constant widths : widths_t := (1, 2, 64, 65, 128, 129, 1024, 1025);
begin
  process
    variable acc, cases, bad : natural := 0;
    variable seed : natural := 12345;

    impure function rnd(n : positive) return natural is
    begin
      seed := (seed * 75 + 74) mod 65537;
      return seed mod n;
    end function;

    -- One case: the four operators against the reference order `e` (-1, 0, 1).
    procedure note(lt, le, gt, ge : boolean; e : integer) is
      variable c : natural := 0;
    begin
      if lt then c := c + 1; end if;
      if le then c := c + 2; end if;
      if gt then c := c + 4; end if;
      if ge then c := c + 8; end if;
      if lt /= (e < 0) or le /= (e <= 0) or gt /= (e > 0) or ge /= (e >= 0) then
        bad := bad + 1;
        report "mismatch in case " & integer'image(cases) & " order " & integer'image(e)
          severity error;
      end if;
      cases := cases + 1;
      acc := (acc * 31 + c + 1) mod 1000003;
    end procedure;

    -- Element k from the left of an array with these bounds.
    function at(l : integer; asc : boolean; k : natural) return integer is
    begin
      if asc then return l + k; else return l - k; end if;
    end function;

    function order(lx, ly : natural; first : integer; lt : boolean) return integer is
    begin
      if first >= 0 then
        if lt then return -1; else return 1; end if;
      elsif lx < ly then return -1;
      elsif lx > ly then return 1;
      end if;
      return 0;
    end function;

    procedure t(x, y : std_ulogic_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : bit_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : boolean_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : string) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : integer_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : big_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    procedure t(x, y : e4_vector) is
      variable f : integer := -1;
      variable lt : boolean := false;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if x(at(x'left, x'ascending, k)) /= y(at(y'left, y'ascending, k)) then
          f := k; lt := x(at(x'left, x'ascending, k)) < y(at(y'left, y'ascending, k)); exit;
        end if;
      end loop;
      note(x < y, x <= y, x > y, x >= y, order(x'length, y'length, f, lt));
    end procedure;

    -- Positions (from the left) of the first difference for width w.
    type pos_t is array (0 to 6) of integer;
    function positions(w : natural) return pos_t is
    begin
      return (0, w / 2, w - 1, 62, 63, 64, w - 65);
    end function;

    procedure slv_cases(w : natural) is
      variable a : std_ulogic_vector(w - 1 downto 0);
      variable r : std_ulogic_vector(0 to w - 1);
      variable b : std_ulogic_vector(w - 1 downto 0);
      variable s : std_ulogic_vector(w + 6 downto 0);
      variable p : integer;
      variable pos : pos_t := positions(w);
    begin
      for i in a'range loop a(i) := std_ulogic'val(rnd(9)); end loop;
      r := a;
      s := "101" & a & "0U-1";
      t(a, a); t(a, r); t(s(w + 3 downto 4), a); t(r, s(w + 3 downto 4));
      for q in pos'range loop
        p := pos(q);
        if p >= 0 and p < w then
          for v in std_ulogic loop
            b := a;
            b(w - 1 - p) := v;
            t(a, b); t(b, r);
            if q < 3 then
              for u in std_ulogic loop
                b(w - 1 - p) := u;
                s(w + 3 - p) := v;
                t(s(w + 3 downto 4), b);
              end loop;
              s(w + 3 downto 4) := a;
            end if;
          end loop;
        end if;
      end loop;
      -- Prefixes and non-prefixes of other lengths, null arrays.
      if w > 1 then
        t(a, a(w - 1 downto 1)); t(a(w - 1 downto 1), a); t(r(0 to w - 2), a);
        b := a; b(w - 1) := 'Z';
        t(a(w - 1 downto 1), b); t(b(w - 2 downto 0), a);
      end if;
      t(a, a(w - 1 downto w)); t(a(0 downto 1), a);
    end procedure;

    procedure bv_cases(w : natural) is
      variable a, b : bit_vector(w - 1 downto 0);
      variable r : bit_vector(1 to w);
      variable bo, co : boolean_vector(0 to w - 1);
      variable pos : pos_t := positions(w);
    begin
      for i in a'range loop a(i) := bit'val(rnd(2)); end loop;
      for i in bo'range loop bo(i) := a(w - 1 - i) = '1'; end loop;
      r := a;
      t(a, r); t(bo, bo); t(a, a(w - 1 downto 1)); t(bo(1 to w - 1), bo);
      t(a(0 downto 1), a);
      for q in pos'range loop
        if pos(q) >= 0 and pos(q) < w then
          b := a; b(w - 1 - pos(q)) := not b(w - 1 - pos(q));
          t(a, b); t(b, r); t(b(w - 1 downto 1), a);
          co := bo; co(pos(q)) := not co(pos(q));
          t(bo, co); t(co, bo); t(co(0 to w - 2), bo);
        end if;
      end loop;
    end procedure;

    procedure scalar_cases(w : natural) is
      variable sa, sb : string(1 to w);
      variable ia, ib : integer_vector(w - 1 downto 0);
      variable ga, gb : big_vector(0 to w - 1);
      variable xa, xb : e4_vector(0 to w - 1);
      variable pos : pos_t := positions(w);
      variable p : integer;
    begin
      for i in 1 to w loop sa(i) := character'val(rnd(256)); end loop;
      for i in ia'range loop ia(i) := rnd(2001) - 1000; end loop;
      for i in ga'range loop ga(i) := big(rnd(2001) - 1000) * 1073741827; end loop;
      for i in xa'range loop xa(i) := e4'val(rnd(4)); end loop;
      t(sa, sa); t(ia, ia); t(ga, ga); t(xa, xa);
      t(sa, sa(1 to w - 1)); t(ia(w - 1 downto 1), ia); t(ga(0 to w - 2), ga); t(xa, xa(0 to w - 2));
      t(sa(2 to 1), sa); t(ia, ia(0 downto 1)); t(xa(1 to 0), xa);
      for q in pos'range loop
        p := pos(q);
        if p >= 0 and p < w then
          sb := sa; sb(1 + p) := character'val((character'pos(sa(1 + p)) + 128) mod 256);
          t(sa, sb); t(sb, sa); t(sb(1 to w - 1), sa);
          ib := ia; ib(w - 1 - p) := -ia(w - 1 - p) - 1;
          t(ia, ib); t(ib, ia); t(ib, ia(w - 1 downto 1));
          gb := ga; gb(p) := -ga(p) - 1;
          t(ga, gb); t(gb, ga);
          xb := xa; xb(p) := e4'val((e4'pos(xa(p)) + 1 + rnd(3)) mod 4);
          t(xa, xb); t(xb, xa); t(xb(0 to w - 2), xa);
        end if;
      end loop;
    end procedure;
  begin
    -- Hand-checked cases.
    assert string'("abc") < "abd" and string'("ab") < "abc" and string'("") < "a"
      and string'("b") > "abc"
      severity failure;
    assert bit_vector'("10") < "11" and bit_vector'("1") > "01" and bit_vector'("0") < "00"
      severity failure;
    assert std_ulogic_vector'("U1") < "X0" and std_ulogic_vector'("-") > "H"
      and std_ulogic_vector'("01") <= "01" and not (std_ulogic_vector'("01") < "01")
      severity failure;
    assert integer_vector'(-5, 3) < (-4, -100) and integer_vector'(0 => 7) < (7, 0)
      and integer_vector'(7, 0) > (0 => 7) and integer_vector'(1 => 3) = (0 => 3)
      severity failure;
    for k in widths'range loop
      slv_cases(widths(k));
      bv_cases(widths(k));
      scalar_cases(widths(k));
    end loop;
    report "array_order cases=" & integer'image(cases) & " acc=" & integer'image(acc)
      & " bad=" & integer'image(bad);
    wait;
  end process;
end architecture;
