-- difftest: top=slot_promotion std=2008
-- Variables, parameters and loop bounds of subprograms that the optimiser keeps in registers
-- (#444): loops with exit and next, reads before writes, real and boolean variables,
-- recursion, a variable reached by a nested subprogram or passed as an inout actual (which
-- must stay in memory), and a scan over an unconstrained string.
entity slot_promotion is
end entity;

architecture a of slot_promotion is
  type int_vec is array (natural range <>) of integer;

  function count_to (n : integer) return integer is
    variable c : integer := 0;
  begin
    while c < n loop
      c := c + 1;
    end loop;
    return c;
  end function;

  function first_of (s : string; ch : character) return integer is
    alias a : string(1 to s'length) is s;
  begin
    for i in a'range loop
      if a(i) = ch then
        return i;
      end if;
    end loop;
    return 0;
  end function;

  function sum_skip (v : int_vec; skip : integer; stop : integer) return integer is
    variable t : integer := 0;
  begin
    for i in v'range loop
      next when v(i) = skip;
      exit when v(i) = stop;
      t := t + v(i);
    end loop;
    return t;
  end function;

  function down (n : integer) return integer is
    variable t : integer := 0;
  begin
    for i in n downto 1 loop
      t := t * 2 + i;
    end loop;
    for i in 1 to 0 loop
      t := -1;
    end loop;
    return t;
  end function;

  function late (sel : boolean) return integer is
    variable x : integer range 5 to 10;
    variable y : integer := 7;
  begin
    if sel then
      y := x + 1;
    end if;
    return y + x;
  end function;

  function fact (n : natural) return natural is
    variable r : natural := 1;
  begin
    if n > 1 then
      r := n * fact(n - 1);
    end if;
    return r;
  end function;

  function mean (v : int_vec) return real is
    variable s : real := 0.0;
    variable k : integer := 0;
  begin
    for i in v'range loop
      s := s + real(v(i));
      k := k + 1;
    end loop;
    if k = 0 then
      return 0.0;
    end if;
    return s / real(k);
  end function;

  function outer (n : integer) return integer is
    variable acc : integer := 0;
    procedure bump (by : integer) is
    begin
      acc := acc + by;
    end procedure;
  begin
    for i in 1 to n loop
      bump(i);
    end loop;
    return acc;
  end function;

  procedure swap (a, b : inout integer) is
    variable t : integer := a;
  begin
    a := b;
    b := t;
  end procedure;

  procedure step (x : inout integer; k : in integer; flag : out boolean) is
  begin
    x := x + k;
    flag := x > 10;
  end procedure;

  function sorted_pair (p, q : integer) return integer is
    variable lo : integer := p;
    variable hi : integer := q;
  begin
    if lo > hi then
      swap(lo, hi);
    end if;
    return lo * 100 + hi;
  end function;
begin
  process
    variable x : integer := 3;
    variable f : boolean;
    constant v : int_vec := (4, 0, 7, 9, 5, 1);
  begin
    assert count_to(0) = 0 and count_to(5) = 5 report "count_to" severity failure;
    assert first_of("hello", 'l') = 3 and first_of("hello", 'z') = 0 and first_of("", 'a') = 0
      report "first_of" severity failure;
    assert sum_skip(v, 0, 9) = 11 and sum_skip(v, 7, 99) = 19 report "sum_skip" severity failure;
    assert down(4) = 49 and down(0) = 0 report "down" severity failure;
    assert late(false) = 12 and late(true) = 11 report "late" severity failure;
    assert fact(5) = 120 and fact(0) = 1 report "fact" severity failure;
    assert mean(v) > 4.33 and mean(v) < 4.34 and mean(v(1 to 0)) = 0.0
      report "mean" severity failure;
    assert outer(4) = 10 report "outer" severity failure;
    assert sorted_pair(9, 2) = 209 and sorted_pair(2, 9) = 209 report "sorted" severity failure;
    for k in 1 to 4 loop
      step(x, k, f);
      report "x " & integer'image(x) & " " & boolean'image(f);
    end loop;
    report "done";
    wait;
  end process;
end architecture;
