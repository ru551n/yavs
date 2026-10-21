-- difftest: top=call_chain std=2008
-- Calls between small subprograms in a loop: scalar and real arguments, narrow types, a
-- constant array, a variable passed by reference to a procedure, a local array passed down a
-- chain (the callee reads its caller's frame), and temporaries made in a callee.
entity call_chain is end;
architecture t of call_chain is
  type iv is array (natural range <>) of integer;
  subtype byte is integer range 0 to 255;
  function inc(a : integer) return integer is begin return a + 1; end function;
  function twice(a : integer) return integer is begin return inc(inc(a)); end function;
  function clamp(a : integer; lo, hi : integer) return integer is
  begin
    if a < lo then return lo; elsif a > hi then return hi; else return a; end if;
  end function;
  function lg(x : natural) return natural is
    variable y : natural := x / 2;
    variable n : natural := 0;
  begin
    while y > 0 loop y := y / 2; n := n + 1; end loop;
    return n;
  end function;
  function halve(x : real) return real is begin return x / 2.0; end function;
  function lgr(x : real) return integer is
    variable y : real := x;
    variable n : integer := 0;
  begin
    if x <= 0.0 then return 0; end if;
    if x >= 1.0 then
      while y >= 2.0 loop y := halve(y); n := n + 1; end loop;
    else
      while y < 1.0 loop y := y * 2.0; n := n - 1; end loop;
    end if;
    return n;
  end function;
  function tobyte(a : integer) return byte is begin return clamp(a, 0, 255); end function;
  function pick(i : integer) return integer is
    constant k : iv(0 to 3) := (10, 20, 30, 40);
  begin
    return k(i mod 4);
  end function;
  function sum(v : iv) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop s := s + v(i); end loop;
    return s;
  end function;
  function local_sum(n : integer) return integer is
    variable v : iv(1 to 4) := (n, n + 1, n + 2, n + 3);
  begin
    return sum(v);
  end function;
  function pad(n : natural) return string is
  begin
    if n = 0 then return "z"; end if;
    return "n" & integer'image(n);
  end function;
  procedure bump(variable a : inout integer; d : integer) is begin a := a + d; end procedure;
  procedure bump2(variable a : inout integer) is begin bump(a, 1); bump(a, 2); end procedure;
begin
  process
    variable s, t : integer := 0;
    variable r : real := 0.0;
  begin
    for i in 1 to 2000 loop
      s := s + twice(i) - inc(i);
      t := t + lg(i) + tobyte(i - 1000);
      r := r + halve(real(i));
      s := s + pick(i) + local_sum(i mod 7);
      bump(s, 3);
      bump2(t);
    end loop;
    assert s = 94000 report "s=" & integer'image(s) severity failure;
    assert t = 246579 report "t=" & integer'image(t) severity failure;
    assert r = 1000500.0 report "r" severity failure;
    assert lgr(1024.0) = 10 and lgr(0.125) = -3 and lgr(1.0) = 0 and lgr(0.0) = 0 severity failure;
    report pad(0) & " " & pad(12) & " " & integer'image(s) & " " & integer'image(t);
    wait;
  end process;
end;
