-- Small functions called in a loop (inlined) and a procedure with an in/out parameter.
entity inline_calls is end;
architecture a of inline_calls is
  function inc(x : integer) return integer is begin return x + 1; end function;
  function twice(x : integer) return integer is begin return inc(inc(x)); end function;
  function clamp(x, lo, hi : integer) return integer is
  begin
    if x < lo then return lo; elsif x > hi then return hi; else return x; end if;
  end function;
  procedure bump(variable v : inout integer; d : integer) is begin v := v + d; end procedure;
begin
  process
    variable acc : integer := 0;
    variable t : integer := 0;
  begin
    for k in 1 to 200 loop
      acc := twice(acc) - 1;
      acc := clamp(acc, -10, 100000);
      bump(t, acc mod 7);
    end loop;
    report "acc=" & integer'image(acc) & " t=" & integer'image(t);
    wait;
  end process;
end;
