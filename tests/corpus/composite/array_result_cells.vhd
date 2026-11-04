-- difftest: top=array_result_cells std=2008 ref=hand
-- ref=hand: sim1 gives a null result another right bound (it keeps left and length); sim2 and yavs
-- keep the result's own bounds (LRM 8.5), and that line is edited to them.
-- Bounds of array results across every path: null and both directions, a result in a heap object
-- that a later allocation must not collect, recursion, nested and protected functions, the memo of
-- pure calls, a result table, calls at elaboration, the body of an `ieee` subprogram, results of
-- other shapes beside them.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

package array_result_pkg is
  subtype byte is std_ulogic_vector(7 downto 0);
  type byte_vec is array (natural range <>) of byte;
  type rec is record a : integer; s : string(1 to 2); end record;
  type int_vec is array (integer range <>) of integer;

  function shape(x : std_ulogic_vector) return integer;
  function cut(x : std_ulogic_vector; a, b : integer) return std_ulogic_vector;
  function make(l, r : integer; down : boolean; v : std_ulogic) return std_ulogic_vector;
  function twice(n : integer) return std_ulogic_vector;
  type prot is protected
    procedure put(x : std_ulogic_vector);
    impure function get return std_ulogic_vector;
    impure function part(h, l : integer) return std_ulogic_vector;
  end protected;
end package;

package body array_result_pkg is
  function shape(x : std_ulogic_vector) return integer is
    variable d : integer := 0;
  begin
    if x'ascending then d := 1; end if;
    return d + 2 * x'length + 100 * x'left + 10000 * x'right;
  end;

  function cut(x : std_ulogic_vector; a, b : integer) return std_ulogic_vector is
  begin
    if x'ascending then return x(a to b); else return x(a downto b); end if;
  end;

  function make(l, r : integer; down : boolean; v : std_ulogic) return std_ulogic_vector is
    variable up : std_ulogic_vector(l to r) := (others => v);
    variable dn : std_ulogic_vector(l downto r) := (others => v);
  begin
    if down then return dn; else return up; end if;
  end;

  function twice(n : integer) return std_ulogic_vector is
    variable r : std_ulogic_vector(2 * n - 1 downto 0) := (others => '0');
  begin
    r(0) := '1';
    return r;
  end;

  type prot is protected body
    variable v : line;
    variable bits : std_ulogic_vector(15 downto 0) := x"0ff1";
    procedure put(x : std_ulogic_vector) is
    begin
      bits(x'length - 1 downto 0) := x;
    end;
    impure function get return std_ulogic_vector is
    begin
      return bits;
    end;
    impure function part(h, l : integer) return std_ulogic_vector is
    begin
      return bits(h downto l);
    end;
  end protected body;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use work.array_result_pkg.all;

entity array_result_cells is
  generic (g_width : integer := shape(twice(3)));
end;

architecture t of array_result_cells is
  constant c_elab : std_ulogic_vector := make(9, 2, true, '1');
  constant c_len : integer := shape(cut(c_elab, 7, 4));
  constant c_tw : std_ulogic_vector := twice(2);
  signal s_len : std_ulogic_vector(g_width mod 7 downto 0) := (others => '0');
  signal s_from : std_ulogic_vector(c_tw'range) := (others => '1');
  shared variable sv : prot;

  -- A result in a heap object the function's own variable names.
  impure function on_heap(n : integer; c : character) return string is
    variable p : line := new string'((1 to n => c));
  begin
    return p.all;
  end;

  impure function churn(n : integer) return integer is
    variable p : line;
  begin
    for i in 1 to n loop
      p := new string(1 to 2000);
      deallocate(p);
    end loop;
    return n;
  end;

  function rev(x : std_ulogic_vector) return std_ulogic_vector is
    alias a : std_ulogic_vector(0 to x'length - 1) is x;
  begin
    if x'length <= 1 then
      return x;
    end if;
    return rev(a(1 to a'right)) & a(0);
  end;

  function with_nested(x : std_ulogic_vector) return std_ulogic_vector is
    function inner(k : integer) return std_ulogic_vector is
      variable r : std_ulogic_vector(x'length + k downto 1) := (others => '0');
    begin
      r(x'length downto 1) := x;
      return r;
    end;
  begin
    return inner(2)(x'length + 1 downto 2);
  end;

  function same_bytes(x : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(x'range) := x;
  begin
    for i in r'range loop
      if r(i) = '0' then r(i) := 'L'; end if;
    end loop;
    return r;
  end;

  -- A result table: a function of one small argument with many arms and one result shape.
  type color is (red, green, blue, cyan, pink, gold, gray, teal);
  function code(c : color) return std_ulogic_vector is
    variable r : std_ulogic_vector(5 downto 3);
  begin
    case c is
      when red => r := "001";
      when green => r := "010";
      when blue => r := "011";
      when cyan => r := "100";
      when pink => r := "101";
      when gold => r := "110";
      when gray => r := "111";
      when teal => r := "000";
    end case;
    return r;
  end;

  function rec_of(n : integer) return rec is
  begin
    return (n, "ab");
  end;

  function bytes_of(x : std_ulogic_vector) return byte_vec is
    variable r : byte_vec(1 to 2) := (x(7 downto 0), x(15 downto 8));
  begin
    return r;
  end;

  function ints(l, r : integer) return int_vec is
    variable v : int_vec(l to r);
  begin
    for i in v'range loop v(i) := i * 2; end loop;
    return v;
  end;

  function blen(v : byte_vec) return integer is
  begin
    return v'length;
  end;

  function ileft(v : int_vec) return integer is
  begin
    return v'left;
  end;

  function ilen(v : int_vec) return integer is
  begin
    return v'length;
  end;

  function fail_shape(x : std_ulogic_vector) return integer is
  begin
    return x'length;
  end;

  function pp(x : std_ulogic_vector) return string is
    variable s : string(1 to x'length);
    variable j : integer := 1;
  begin
    for i in x'range loop
      s(j) := std_ulogic'image(x(i))(2);
      j := j + 1;
    end loop;
    return s;
  end;
begin
  main : process
    variable a : std_ulogic_vector(15 downto 0) := x"a5c3";
    variable b : std_ulogic_vector(0 to 15) := x"a5c3";
    variable m : integer := 0;
    variable l : line;
    variable u : unsigned(7 downto 0) := x"0f";
    variable w : std_ulogic_vector(3 downto 0);
    variable r0 : rec;
  begin
    report "elab " & integer'image(c_len) & " " & integer'image(c_elab'left) & " " & integer'image(g_width);
    report "signal " & integer'image(s_len'left) & " " & integer'image(s_from'left);

    -- Bounds of results, read after the call.
    report "make " & integer'image(shape(make(3, 6, false, '1'))) & " " & integer'image(shape(make(6, 3, true, '0')));
    report "null " & integer'image(shape(make(6, 3, false, '1'))) & " " & integer'image(shape(make(3, 6, true, '0')));
    report "cut " & pp(cut(a, 7, 2)) & " " & pp(cut(b, 2, 7));
    report "cut of cut " & pp(cut(cut(a, 12, 2), 9, 4)) & " " & pp(cut(cut(b, 2, 12), 4, 9));
    report "result slice " & pp(make(7, 0, true, '1')(5 downto 2));
    report "result elem " & std_ulogic'image(cut(a, 7, 2)(3));
    report "concat " & pp(cut(a, 3, 0) & cut(b, 0, 3));
    report "in if " & boolean'image(cut(a, 3, 0) = "0011");

    -- Held across another call, and a statement later.
    w := cut(a, 3, 0);
    report "assign " & pp(w);
    report "two " & integer'image(shape(cut(a, 7, 4)) + shape(cut(b, 8, 11)));

    -- A result in a heap object, while allocation goes on.
    for i in 1 to 4 loop
      report on_heap(5 + i, 'x') & integer'image(churn(20)) & on_heap(3, 'y');
    end loop;

    -- Recursion, nesting, a method.
    report "rev " & pp(rev(a(7 downto 0))) & " " & integer'image(shape(rev(a(7 downto 0))));
    report "rev2 " & integer'image(shape(rev(rev(b(0 to 4)))));
    report "nested " & pp(with_nested(b(2 to 7))) & " " & integer'image(shape(with_nested(a(9 downto 4))));
    sv.put("1010");
    report "method " & pp(sv.get) & " " & integer'image(shape(sv.get)) & " " & pp(sv.part(5, 2));

    -- The memo: the same bytes under other bounds give their own result bounds.
    for t in 1 to 200 loop
      m := m + shape(same_bytes(b(4 to 7))) + shape(same_bytes(a(7 downto 4)));
      m := m + shape(same_bytes(b(8 to 11)));
    end loop;
    report "memo " & integer'image(m) & " " & pp(same_bytes(a(3 downto 0)));

    -- A result table, every value.
    for c in color loop
      report color'image(c) & " " & pp(code(c)) & " " & integer'image(shape(code(c)));
    end loop;

    -- An `ieee` subprogram: its native, and its body where it gives the call back.
    report "numeric " & to_hstring(u + 1) & " " & to_hstring(resize(u, 12)) & " " & integer'image(shape(std_ulogic_vector(to_unsigned(5, 6))));
    report "metavalue " & pp(std_ulogic_vector(unsigned'("1X0") + 1));
    report "logic " & pp(to_stdlogicvector(bit_vector'("1010")));
    report "shift " & to_hstring(shift_left(u, 2)) & " " & to_hstring(u sll 1);

    -- Other shapes beside them.
    r0 := rec_of(3);
    report "rec " & integer'image(r0.a) & r0.s;
    report "bytes " & to_hstring(bytes_of(a)(1)) & " " & integer'image(blen(bytes_of(a)));
    report "ints " & integer'image(ints(-2, 2)(-1)) & " " & integer'image(ileft(ints(-2, 2)));
    report "ints null " & integer'image(ilen(ints(2, -2)));

    wait for 0 ns;
    assert fail_shape(make(4, 1, false, '1')) = 0 severity failure;
    wait;
  end process;
end;
