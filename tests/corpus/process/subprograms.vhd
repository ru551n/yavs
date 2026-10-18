-- difftest: top=subprograms std=2008
entity subprograms is end;
architecture t of subprograms is
  function fib(n : natural) return natural is begin
    if n < 2 then return n; end if;
    return fib(n - 1) + fib(n - 2);
  end;
  procedure inc(variable v : inout integer; by : in integer := 1) is begin v := v + by; end;
  procedure outp(x : out integer; y : inout integer) is begin x := y * 2; y := 0; end;
  pure function "+"(a : string; b : integer) return string is begin return a & integer'image(b); end;
  signal s : integer := 0;
  procedure bump(signal sg : out integer; v : integer) is begin sg <= v; end;
begin
  process
    variable v, a, b : integer := 1;
  begin
    report "fib(15)=" & integer'image(fib(15));
    inc(v); inc(v, 5); report "v=" & integer'image(v);
    b := 21; outp(a, b); report "a=" & integer'image(a) & " b=" & integer'image(b);
    report "tag " + 7;
    bump(s, 9); wait for 1 ns; report "s=" & integer'image(s);
    wait;
  end process;
end;
