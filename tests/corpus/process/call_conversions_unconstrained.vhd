-- difftest: top=call_conversions_unconstrained std=2008 ref=hand
-- Conversions on unconstrained formals of subprograms (LRM 5.3.2.2, 6.5.7.1): the formal
-- takes its index ranges from the conversion: the type mark or the conversion function's
-- parameter subtype in the formal part (`b4(x) => v`, `rev4(x) => v`), the result subtype of
-- the actual part for inout.
-- ref=hand: the reference simulators reject these or give the formal the actual's bounds;
-- the expected output is the LRM result.
entity call_conversions_unconstrained is end;
architecture t of call_conversions_unconstrained is
  subtype b4 is bit_vector(5 to 8);
  function rev4(x : b4) return bit_vector is
    variable r : bit_vector(5 to 8);
  begin
    for k in 0 to 3 loop r(5 + k) := x(8 - k); end loop;
    return r;
  end function;
  function id4(x : bit_vector) return b4 is begin return x; end function;
  procedure setv(variable x : out bit_vector) is
  begin
    report "setv " & integer'image(x'left) & " to " & integer'image(x'right);
    x := (x'range => '1');
    x(x'left) := '0';
  end procedure;
  procedure incv(variable x : inout bit_vector) is
  begin
    report "incv " & integer'image(x'left) & " to " & integer'image(x'right);
    x(x'right) := not x(x'right);
  end procedure;
begin
  process
    variable v : bit_vector(1 to 4) := "0000";
  begin
    setv(b4(x) => v);
    report "v=" & to_string(v);
    v := "1100";
    setv(rev4(x) => v);
    report "v=" & to_string(v);
    v := "0011";
    incv(rev4(x) => id4(v));
    report "v=" & to_string(v);
    wait;
  end process;
end architecture;
