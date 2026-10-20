-- difftest: top=tb_call_frames std=2008
-- Subprogram frames: recursion (frames deeper than a few), calls nested in arguments,
-- arguments of every scalar width, a record and an array, a function reading its parent's
-- variables from several frames up, a procedure waiting inside a recursion, and results
-- used right after the return.
entity tb_call_frames is
end entity;

architecture sim of tb_call_frames is
  type rec is record
    a : integer;
    b : boolean;
    c : character;
  end record;
  type int_vec is array (natural range <>) of integer;

  function fact(n : natural) return natural is
  begin
    if n <= 1 then
      return 1;
    end if;
    return n * fact(n - 1);
  end function;

  function fib(n : natural) return natural is
  begin
    if n < 2 then
      return n;
    end if;
    return fib(n - 1) + fib(n - 2);
  end function;

  function mix(a : integer; b : boolean; c : character; d : real; e : time) return integer is
    variable r : integer := a;
  begin
    if b then
      r := r + character'pos(c);
    end if;
    return r + integer(d) + (e / 1 ns);
  end function;

  function rec_sum(r : rec) return integer is
  begin
    return r.a + character'pos(r.c) + boolean'pos(r.b);
  end function;

  function vec_sum(v : int_vec) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;

  signal s_int : integer := 0;

  procedure wait_down(signal clk : in bit; n : natural) is
  begin
    if n > 0 then
      wait until clk = '1';
      wait_down(clk, n - 1);
    end if;
  end procedure;

  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns else clk;

  main : process
    variable total : integer := 0;
    variable acc   : integer := 7;

    impure function bump(x : integer) return integer is
    begin
      acc := acc + 1;
      return x + acc;
    end function;

    impure function deep(n : natural) return integer is
    begin
      if n = 0 then
        return acc;
      end if;
      return deep(n - 1) + bump(0) - acc + 1;
    end function;

    procedure add_to(variable t : inout integer; v : integer) is
    begin
      t := t + v;
    end procedure;
  begin
    assert fact(10) = 3628800 report "fact" severity failure;
    assert fib(15) = 610 report "fib" severity failure;
    assert mix(1, true, 'A', 2.0, 3 ns) = 1 + 65 + 2 + 3 report "mix" severity failure;
    assert mix(1, false, 'A', 2.0, 3 ns) = 1 + 2 + 3 report "mix false" severity failure;
    assert rec_sum((a => 5, b => true, c => 'B')) = 5 + 66 + 1 report "rec" severity failure;
    assert vec_sum((1, 2, 3, 4)) = 10 report "vec" severity failure;
    assert fact(fib(5)) = 120 report "nested args" severity failure;
    assert fib(fact(3)) = 8 report "nested args 2" severity failure;
    report "deep " & integer'image(deep(50));
    for i in 1 to 100 loop
      add_to(total, fib(10) - fact(5));
    end loop;
    assert total = -6500 report "loop: " & integer'image(total) severity failure;
    report "pure calls ok";
    wait_down(clk, 5);
    assert now = 45 ns report "wait in recursion" severity failure;
    s_int <= bump(1);
    wait for 1 ns;
    assert s_int = 59 report "after wait: " & integer'image(s_int) severity failure;
    report "frames ok " & integer'image(acc);
    wait;
  end process;
end architecture;
