-- difftest: top=local_aggregate_init std=2008
-- Locals of arrays of logic arrays and records initialised with aggregates, in functions and
-- procedures called again and again: static and run-time bounds, ascending, descending and null
-- ranges, rows of std_ulogic, bit and boolean, `others` with a named override, rows that are not
-- one literal, a value taken from a slice whose rows do not fill whole plane words, allocators.
-- Each call must start from the initial value, whatever the call before wrote.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity local_aggregate_init is end;

architecture t of local_aggregate_init is
  type t_arr is array (natural range <>) of signed(11 downto 0);
  type t_bits is array (integer range <>) of bit_vector(0 to 4);
  type t_bools is array (natural range <>) of boolean_vector(2 downto 0);
  type t_wide is array (natural range <>) of std_logic_vector(299 downto 0);
  type t_rec is record
    rows : t_arr(0 to 2);
    v    : std_logic_vector(6 downto 0);
    n    : integer;
  end record;
  type t_arr_ptr is access t_arr;

  signal s : t_arr(0 to 2) := (others => (others => '0'));
  signal events : natural := 0;

  function sum (a : t_arr) return integer is
    variable r : integer := 0;
  begin
    for i in a'range loop
      r := r + to_integer(a(i)) * (i + 1);
    end loop;
    return r;
  end function;

  -- Static bounds: the initial value is a constant.
  function f_static (k : integer) return integer is
    variable r : t_arr(0 to 5) := (others => (others => '0'));
  begin
    r(k mod 6) := to_signed(k, 12);
    return sum(r);
  end function;

  -- Run-time bounds, any direction, possibly null.
  function f_dyn (lo, hi : integer; up : boolean; k : integer) return integer is
    variable ru : t_arr(lo to hi) := (others => (others => '1'));
    variable rd : t_arr(hi downto lo) := (others => (others => '0'));
  begin
    if ru'length > 0 then
      ru(lo + k mod ru'length) := to_signed(k, 12);
      rd(lo + (k + 1) mod rd'length) := to_signed(-k, 12);
    end if;
    if up then
      return sum(ru) + ru'length;
    else
      return sum(rd) + 7 * rd'length;
    end if;
  end function;

  -- `others` and a named override; rows that are not one literal.
  function f_mix (n : natural; k : integer) return integer is
    variable a : t_arr(0 to n - 1) := (1 => x"0FF", others => (others => '1'));
    variable b : t_arr(n - 1 downto 0) := (others => "010101010101");
  begin
    a(k mod n) := a(k mod n) + 1;
    b(k mod n) := b(k mod n) - 1;
    return sum(a) + 3 * sum(b);
  end function;

  -- Rows of bit, boolean and a row over several plane words.
  procedure p_kinds (n : natural; k : integer; r : inout integer) is
    variable b  : t_bits(-1 to n - 2) := (others => (others => '1'));
    variable bo : t_bools(0 to n - 1) := (others => (others => true));
    variable w  : t_wide(0 to n - 1) := (others => (others => 'H'));
  begin
    b(k mod n - 1)(k mod 5) := '0';
    bo(k mod n)(k mod 3) := false;
    w(k mod n)(k mod 300) := 'L';
    for i in b'range loop
      for j in b(i)'range loop
        if b(i)(j) = '1' then r := r + i + 2 * j; end if;
      end loop;
    end loop;
    for i in bo'range loop
      for j in bo(i)'range loop
        if bo(i)(j) then r := r + 3 * i + j; end if;
      end loop;
    end loop;
    for i in w'range loop
      for j in 0 to 299 loop
        if w(i)(j) = 'L' then r := r + 5 * i + j; end if;
        assert w(i)(j) = 'H' or w(i)(j) = 'L' severity failure;
      end loop;
    end loop;
  end procedure;

  -- A local initialised from a slice (rows of 12 elements: not whole plane words).
  function f_slice (src : t_arr; k : natural) return integer is
    variable v : t_arr(0 to 2) := src(k to k + 2);
    variable w : t_arr(0 to 2) := (src(k), src(k + 1), src(k + 2));
  begin
    assert v = w report "slice copy differs" severity failure;
    v(1) := v(1) + 1;
    assert v /= w severity failure;
    return sum(v);
  end function;

  -- A record whose array-of-logic-arrays field has an aggregate default.
  function f_rec (k : integer) return integer is
    variable r : t_rec := (rows => (others => (others => '0')), v => "1010101", n => 3);
  begin
    r.rows(k mod 3) := to_signed(k, 12);
    r.n := r.n + k;
    return sum(r.rows) + r.n + to_integer(unsigned(r.v));
  end function;

begin
  watch : process (s) is
  begin
    events <= events + 1;
  end process;

  main : process is
    variable acc : integer := 0;
    variable big : t_arr(0 to 9);
    variable p : t_arr_ptr;
  begin
    for i in big'range loop
      big(i) := to_signed(37 * i - 100, 12);
    end loop;
    for k in 0 to 11 loop
      acc := acc + f_static(k);
      acc := acc + f_dyn(2, 2 + k mod 4, true, k) + f_dyn(3, 3 + k mod 5, false, k);
      acc := acc + f_dyn(5, 4, true, k) + f_dyn(5, 4, false, k);
      acc := acc + f_mix(2 + k mod 4, k);
      p_kinds(1 + k mod 3, k, acc);
      acc := acc + f_slice(big, k mod 8);
      acc := acc + f_rec(k);
    end loop;
    report "acc = " & integer'image(acc);
    p := new t_arr'(big(3 to 5));
    assert p.all = big(3 to 5) severity failure;
    p.all(3) := p.all(3) + 1;
    assert big(3) = to_signed(37 * 3 - 100, 12) severity failure;
    report "p sum = " & integer'image(sum(p.all));
    deallocate(p);
    -- Driving the same value again makes no event; a different one does.
    s <= big(4 to 6);
    wait for 1 ns;
    s <= big(4 to 6);
    wait for 1 ns;
    s <= big(5 to 7);
    wait for 1 ns;
    report "events = " & integer'image(events);
    report "done";
    wait;
  end process;
end architecture;
