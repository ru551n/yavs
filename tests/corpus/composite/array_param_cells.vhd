-- difftest: top=array_param_cells std=2008 ref=hand
-- ref=hand: sim1 gives a null slice returned by a function another right bound (it keeps left and
-- length); sim2 and yavs keep the slice's own bounds (LRM 8.5), and the two lines are edited to that.
-- Bounds of array parameters and results across every call path: null ranges, both directions,
-- negative bounds, resolved elements, 'range of a parameter, slices of slices, a parameter's bounds
-- read after a nested call and from a nested subprogram, the same bytes with other bounds through
-- the memo of pure calls, out and inout slices, a procedure that waits, a subprogram generic.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity array_param_cells is
  generic (g_hi : integer := 12; g_lo : integer := 3);
end;

architecture t of array_param_cells is
  subtype row_t is unsigned(g_hi - g_lo downto 0);
  type int_vec is array (integer range <>) of integer;
  type rec is record a : integer; b : bit; end record;
  type rec_vec is array (natural range <>) of rec;
  subtype byte is std_ulogic_vector(7 downto 0);
  type byte_vec is array (integer range <>) of byte;

  function resolve_sum(v : int_vec) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop s := s + v(i); end loop;
    return s;
  end;
  subtype rint is resolve_sum integer;
  type rint_vec is array (natural range <>) of rint;

  -- A digest of every bound word a parameter shows, so a wrong word changes the result.
  function shape(x : std_ulogic_vector) return integer is
    variable d : integer := 0;
  begin
    if x'ascending then d := 1; end if;
    return d + 2 * x'length + 100 * x'left + 10000 * x'right;
  end;

  function get_row(addr : std_ulogic_vector) return row_t is
  begin
    return unsigned(addr(g_hi downto g_lo));
  end;

  function slice_up(x : std_ulogic_vector; a, b : integer) return std_ulogic_vector is
  begin
    return x(a to b);
  end;

  function slice_dn(x : std_ulogic_vector; a, b : integer) return std_ulogic_vector is
  begin
    return x(a downto b);
  end;

  function same(x : std_ulogic_vector) return std_ulogic_vector is
  begin
    return x;
  end;

  function rev(x : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(x'reverse_range);
  begin
    for i in x'range loop r(i) := x(i); end loop;
    return r;
  end;

  function local_copy(x : std_ulogic_vector) return integer is
    variable v : std_ulogic_vector(x'range) := x;
    variable w : std_ulogic_vector(1 to x'length) := x;
    alias a : std_ulogic_vector(x'length - 1 downto 0) is x;
    variable n : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then n := n + 1; end if;
    end loop;
    for i in w'range loop
      if w(i) = '1' then n := n + 10; end if;
    end loop;
    for i in a'range loop
      if a(i) = '1' then n := n + 100; end if;
    end loop;
    return n + shape(v) + shape(w) + shape(a);
  end;

  -- The bounds after a nested call and from a nested subprogram.
  function after_call(x : std_ulogic_vector) return integer is
    variable n : integer := 0;
    function inner(k : integer) return integer is
    begin
      return k + x'left - x'right + x'length;
    end;
    function inner2(y : std_ulogic_vector) return integer is
    begin
      return shape(y) + x'length + inner(y'length);
    end;
  begin
    n := shape(x(x'range));
    n := n + inner(1);
    if x'length > 2 then
      if x'ascending then
        n := n + inner2(x(x'left + 1 to x'right - 1));
      else
        n := n + inner2(x(x'left - 1 downto x'right + 1));
      end if;
    end if;
    return n + x'left + x'length;
  end;

  function depth(x : std_ulogic_vector) return integer is
  begin
    if x'length = 0 then
      return 0;
    elsif x'ascending then
      return 1 + depth(x(x'left + 1 to x'right));
    else
      return 1 + depth(x(x'left - 1 downto x'right));
    end if;
  end;

  -- Negative bounds, in an element type of the logic family.
  type sul is array (integer range <>) of std_ulogic;

  function sshape(x : sul) return integer is
    variable n : integer := x'length * 2;
  begin
    if x'ascending then n := n + 1; end if;
    for i in x'range loop
      if x(i) = '1' then n := n + (i + 5) * 100; end if;
    end loop;
    return n + 1000 * x'left + 100000 * x'right;
  end;

  function scut(x : sul; a, b : integer) return sul is
  begin
    if x'ascending then return x(a to b); else return x(a downto b); end if;
  end;

  function sum_int(v : int_vec) return integer is
    variable s : integer := v'left * 3 + v'right * 5;
  begin
    for i in v'range loop s := s + v(i); end loop;
    return s;
  end;

  function middle(v : int_vec) return int_vec is
  begin
    return v(v'left + 1 to v'right - 1);
  end;

  function sum_rec(v : rec_vec) return integer is
    variable s : integer := v'left;
  begin
    for i in v'range loop
      s := s + v(i).a;
      if v(i).b = '1' then s := s + 1000; end if;
    end loop;
    return s;
  end;

  function sum_res(v : rint_vec) return integer is
    variable s : integer := v'length;
  begin
    for i in v'reverse_range loop s := s * 2 + v(i); end loop;
    return s;
  end;

  function logic_res(v : std_logic_vector) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then s := s + 2 ** (i - v'right); elsif v(i) = 'Z' then s := s + 1000; end if;
    end loop;
    return s + v'length * 100000;
  end;

  function bytes(v : byte_vec) return integer is
    variable s : integer := v'left;
  begin
    for i in v'range loop
      s := s + to_integer(unsigned(v(i)(5 downto 2))) + v(i)'left;
    end loop;
    return s;
  end;

  function str(s : string) return integer is
  begin
    return s'left + s'right * 10 + s'length * 100 + character'pos(s(s'right));
  end;

  -- Constrained formals, static and not.
  function fixed(x : std_ulogic_vector(3 downto 0)) return integer is
  begin
    return shape(x) + x'left;
  end;

  function sized(x : std_ulogic_vector(g_lo downto 0)) return integer is
  begin
    return shape(x) + x'left;
  end;

  procedure copy_out(x : out std_ulogic_vector; v : std_ulogic) is
  begin
    for i in x'range loop x(i) := v; end loop;
  end;

  procedure swap_ends(x : inout std_ulogic_vector) is
    variable t : std_ulogic;
  begin
    if x'length > 1 then
      t := x(x'left);
      x(x'left) := x(x'right);
      x(x'right) := t;
    end if;
  end;

  procedure bump(x : inout unsigned; n : out integer) is
  begin
    x := x + 1;
    n := x'length + x'left;
  end;

  -- A pure function worth remembering: same bytes, other bounds, other result.
  function memo_me(x : std_ulogic_vector) return integer is
    variable n : integer := 0;
  begin
    for i in x'range loop
      if x(i) = '1' then n := n + 1; end if;
    end loop;
    return n + x'left * 3 + x'right * 7;
  end;

  function memo_ret(x : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(x'length - 1 downto 0) := x;
  begin
    for i in r'range loop r(i) := not r(i); end loop;
    return r;
  end;

  signal s_data : std_logic_vector(15 downto 0) := x"a5c3";
  signal s_rint : rint_vec(2 to 5) := (1, 2, 3, 4);
  signal s_res : std_logic_vector(3 downto 0);

  -- A signal parameter's descriptor stays a pointer.
  function sig_shape(signal x : std_logic_vector) return integer is
  begin
    return x'left * 2 + x'right * 3 + x'length;
  end;

  procedure waits(x : in std_ulogic_vector; signal clk : in std_ulogic; n : out integer) is
    variable a : integer := x'left;
  begin
    wait until clk = '1';
    a := a + x'right * 10;
    wait until clk = '0';
    n := a + x'length * 100 + shape(x(x'range));
  end;

  signal clk : std_ulogic := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 1 ns when not done;
  s_res <= "1Z00";
  s_res <= "Z1Z0";

  main : process
    variable a : std_ulogic_vector(15 downto 0) := x"a5c3";
    variable b : std_ulogic_vector(0 to 15) := x"a5c3";
    variable n : std_ulogic_vector(3 downto 4);
    variable u : std_ulogic_vector(4 to 3);
    variable neg : std_ulogic_vector(3 to 10) := x"96";
    variable negd : std_ulogic_vector(10 downto 3) := x"69";
    variable sn : sul(-3 to 4) := x"96";
    variable sd : sul(4 downto -3) := x"69";
    variable iv : int_vec(-2 to 3) := (1, 2, 3, 4, 5, 6);
    variable rv : rec_vec(1 to 3) := ((1, '0'), (2, '1'), (3, '0'));
    variable bv : byte_vec(4 downto 1) := (x"11", x"22", x"33", x"44");
    variable r : row_t;
    variable k, m : integer := 0;
    variable un : unsigned(7 downto 0) := x"fe";
    variable txt : string(1 to 3) := "abc";
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
    -- Null ranges, both directions, passed on and sliced.
    report "null " & integer'image(shape(n)) & " " & integer'image(shape(u));
    report "null slice " & integer'image(shape(a(3 downto 4))) & " " & integer'image(shape(b(4 to 3)));
    report "null of null " & integer'image(shape(slice_dn(n, 2, 3))) & " " & integer'image(shape(slice_up(u, 7, 2)));
    report "null local " & integer'image(local_copy(n)) & " " & integer'image(local_copy(u));
    report "null rev " & integer'image(shape(rev(n))) & " " & integer'image(shape(rev(u)));
    report "null depth " & integer'image(depth(n)) & " " & integer'image(depth(u));
    report "null after " & integer'image(after_call(u));

    -- Both directions, negative bounds.
    for t in 1 to 3 loop
      report "shape " & integer'image(shape(a)) & " " & integer'image(shape(b)) & " " &
        integer'image(shape(neg)) & " " & integer'image(shape(negd));
      report "local " & integer'image(local_copy(a)) & " " & integer'image(local_copy(b)) & " " &
        integer'image(local_copy(neg)) & " " & integer'image(local_copy(negd));
      report "after " & integer'image(after_call(a)) & " " & integer'image(after_call(b)) & " " &
        integer'image(after_call(neg)) & " " & integer'image(after_call(negd));
      report "depth " & integer'image(depth(a)) & " " & integer'image(depth(neg));
    end loop;

    report "neg " & integer'image(sshape(sn)) & " " & integer'image(sshape(sd));
    report "neg slice " & integer'image(sshape(scut(sn, -2, 1))) & " " & integer'image(sshape(scut(sd, 1, -2)));
    report "neg null " & integer'image(sshape(scut(sn, 1, -2))) & " " & integer'image(sshape(scut(sd, -2, 1)));
    report "neg slice of slice " & integer'image(sshape(scut(scut(sn, -3, 2), -1, 0)));

    -- Results: their own bounds, slices of slices.
    r := get_row(a);
    report "get_row " & to_hstring(r);
    r := get_row(same(a));
    report "get_row same " & to_hstring(r);
    report "slice " & pp(slice_up(b, 2, 7)) & " " & pp(slice_dn(a, 7, 2));
    report "slice shape " & integer'image(shape(slice_up(b, 2, 7))) & " " & integer'image(shape(slice_dn(a, 7, 2)));
    report "slice of slice " & integer'image(shape(slice_dn(a, 9, 2)(7 downto 4)));
    report "slice of slice " & pp(slice_dn(a, 9, 2)(7 downto 4));
    report "slice of slice2 " & integer'image(shape(a(12 downto 1)(9 downto 3)(8 downto 4)));
    report "same " & integer'image(shape(same(neg))) & " " & pp(same(same(neg)));
    report "same slice " & integer'image(shape(same(negd(8 downto 5)))) & " " & pp(same(negd(8 downto 5)));
    report "rev " & integer'image(shape(rev(a))) & " " & pp(rev(a)) & " " & pp(rev(b(3 to 9)));
    report "literal " & integer'image(shape("1010")) & " " & integer'image(shape(std_ulogic_vector'("01")));
    report "concat " & integer'image(shape(a(3 downto 0) & b(0 to 1)));
    report "concat result " & pp(same(a(3 downto 0)) & same(b(0 to 1)));

    -- Elements of other kinds.
    report "int " & integer'image(sum_int(iv)) & " " & integer'image(sum_int(iv(0 to 2))) & " " & integer'image(sum_int(iv(1 to 0)));
    report "int mid " & integer'image(sum_int(middle(iv))) & " " & integer'image(sum_int(middle(middle(iv))));
    report "rec " & integer'image(sum_rec(rv)) & " " & integer'image(sum_rec(rv(2 to 3)));
    report "bytes " & integer'image(bytes(bv)) & " " & integer'image(bytes(bv(3 downto 2)));
    report "byte elem " & integer'image(shape(bv(2))) & " " & integer'image(shape(bv(3)(6 downto 1)));
    report "str " & integer'image(str("hello")) & " " & integer'image(str(txt(2 to 3)));

    -- Constrained formals.
    report "fixed " & integer'image(fixed(a(7 downto 4))) & " " & integer'image(fixed(b(2 to 5)));
    report "sized " & integer'image(sized(a(11 downto 8))) & " " & integer'image(sized(b(4 to 7)));

    -- Writes through parameters: whole, slices, slices of slices.
    copy_out(a(3 downto 0), '0');
    report "out " & pp(a);
    copy_out(b(0 to 3), '1');
    report "out " & pp(b);
    swap_ends(a(15 downto 8));
    report "inout " & pp(a);
    swap_ends(neg);
    report "inout " & pp(neg);
    swap_ends(a(11 downto 2)(9 downto 4));
    report "inout " & pp(a);
    bump(un(3 downto 0), k);
    report "bump " & to_hstring(un) & " " & integer'image(k);
    bump(un, k);
    report "bump " & to_hstring(un) & " " & integer'image(k);

    -- The memo: same bytes under other bounds, and a result with its own.
    for t in 1 to 200 loop
      m := m + memo_me(b(4 to 7)) + memo_me(a(7 downto 4));
      m := m + memo_me(b(8 to 11));
      assert shape(memo_ret(b(4 to 7))) = shape(a(3 downto 0)) severity failure;
    end loop;
    report "memo " & integer'image(m);

    -- Signals as `in` actuals, resolved elements, a signal parameter.
    wait for 0 ns;
    report "sig " & integer'image(logic_res(s_data)) & " " & integer'image(logic_res(s_data(7 downto 2)));
    report "res " & integer'image(logic_res(s_res)) & " " & integer'image(logic_res(s_res(2 downto 1)));
    report "rint " & integer'image(sum_res(s_rint)) & " " & integer'image(sum_res(s_rint(3 to 4)));
    report "sig param " & integer'image(sig_shape(s_data)) & " " & integer'image(sig_shape(s_data(9 downto 3)));

    -- Bounds read after a suspension.
    waits(a(7 downto 2), clk, k);
    report "wait " & integer'image(k);
    waits(b(3 to 9), clk, k);
    report "wait " & integer'image(k);

    done <= true;
    wait;
  end process;
end;
