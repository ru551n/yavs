-- difftest: top=wait_proc_locals std=2008
-- Locals of procedures that wait, read and written on every side of the wait: a local read
-- after the wait, a wait inside a loop with locals live around the jump back, a local live
-- across the call of a procedure that waits, an `in` parameter read after the wait, locals
-- dead at the wait (worked on before it, assigned again after it).
entity wait_proc_locals is
end entity;

architecture sim of wait_proc_locals is
  signal clk : bit := '0';
  signal tick : integer := 0;

  -- `a` is live across the wait, `b` is dead there (assigned again after it).
  procedure across(variable r : inout integer; n : integer) is
    variable a : integer := 3;
    variable b : integer := 5;
  begin
    for i in 1 to 3 loop
      a := a * 7 + n + i;
      b := b + a mod 11;
    end loop;
    r := r + b;
    wait until clk = '1';
    b := 1;
    for i in 1 to 2 loop
      b := b * 3 + a mod 13 + n;
    end loop;
    r := (r + a + b) mod 100003;
  end procedure;

  -- A wait in a loop: `s` and `k` are live around the jump back and across the wait.
  procedure cycles(variable r : inout integer; n : integer) is
    variable s : integer := 0;
    variable k : integer := 0;
  begin
    while k < n loop
      s := s + k * k + tick;
      wait until clk = '0';
      k := k + 1;
      s := s mod 7919;
    end loop;
    r := (r + s) mod 100003;
  end procedure;

  -- `c` is live across the call of `across`, which waits; `n` is read after it.
  procedure outer(variable r : inout integer; n : integer) is
    variable c : integer := 0;
  begin
    for i in 1 to 4 loop
      c := c + i * n;
    end loop;
    across(r, n + c mod 5);
    c := c + n;
    r := (r * 3 + c) mod 100003;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 1 us;

  counter : process (clk)
  begin
    if clk = '1' then
      tick <= tick + 1;
    end if;
  end process;

  main : process
    variable r : integer := 1;
  begin
    for k in 1 to 5 loop
      across(r, k);
    end loop;
    report "across r=" & integer'image(r) & " now=" & time'image(now);
    for k in 1 to 4 loop
      cycles(r, k);
    end loop;
    report "cycles r=" & integer'image(r) & " now=" & time'image(now);
    for k in 1 to 5 loop
      outer(r, k);
    end loop;
    report "outer r=" & integer'image(r) & " now=" & time'image(now);
    wait;
  end process;
end architecture;
