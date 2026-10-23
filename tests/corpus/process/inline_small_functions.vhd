-- difftest: top=inline_small_functions std=2008 exit=1
-- Small subprograms with branches, a case, recursion, a protected object and byte variables that
-- the optimiser copies into their callers: results, the order of reports and runtime errors are
-- the calls' own.
entity inline_small_functions is end entity;

architecture test of inline_small_functions is
  type mode_t is (alpha, beta, gamma, delta);

  function pick(m : mode_t; a, b : integer) return integer is
  begin
    case m is
      when alpha => return a + b;
      when beta | gamma => return a - b;
      when others => return a * b;
    end case;
  end function;

  function sign(x : integer) return integer is
  begin
    if x > 0 then return 1; elsif x < 0 then return -1; else return 0; end if;
  end function;

  function fact(n : natural) return natural is
  begin
    if n <= 1 then return 1; end if;
    return n * fact(n - 1);
  end function;

  function even(n : natural) return boolean;
  function odd(n : natural) return boolean is
  begin
    if n = 0 then return false; end if;
    return even(n - 1);
  end function;
  function even(n : natural) return boolean is
  begin
    if n = 0 then return true; end if;
    return odd(n - 1);
  end function;

  function wrap(x : integer) return integer is
  begin
    return pick(mode_t'val(x mod 4), x, sign(x) + 3);
  end function;

  function flag_of(x : integer) return boolean is
    variable b : boolean := false;
    variable c : character := 'a';
  begin
    if x > 3 then b := true; c := 'z'; end if;
    return b and c /= 'a';
  end function;

  function checked(x : integer) return integer is
  begin
    if x > 0 then return 10 / x; else return 100 / x; end if;
  end function;

  type counter_t is protected
    procedure bump(by : integer);
    impure function get return integer;
    impure function twice return integer;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    procedure bump(by : integer) is
    begin
      if by > 0 then n := n + by; else n := n - 1; end if;
    end procedure;
    impure function get return integer is
    begin
      return n;
    end function;
    impure function twice return integer is
    begin
      return get * 2;
    end function;
  end protected body;
  shared variable c : counter_t;
begin
  process
    variable acc : integer := 0;
  begin
    for i in -3 to 9 loop
      acc := acc + wrap(i);
      c.bump(i);
    end loop;
    report "acc=" & integer'image(acc) & " c=" & integer'image(c.get) & " tw=" & integer'image(c.twice);
    report "fact=" & integer'image(fact(7)) & " even=" & boolean'image(even(10)) & " odd=" & boolean'image(odd(7));
    for i in 0 to 6 loop
      report integer'image(i) & " " & boolean'image(flag_of(i));
    end loop;
    report "checked " & integer'image(checked(5));
    report "checked " & integer'image(checked(0));
    report "not reached";
    wait;
  end process;
end architecture;
