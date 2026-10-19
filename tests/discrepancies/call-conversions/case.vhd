-- (from the differential corpus)
-- Conversions in subprogram call associations (LRM 6.5.7.1): for an out formal the formal
-- part converts the formal's value to the actual's type after the call; for inout the
-- actual part also converts the actual to the formal's type before it.
-- ref=hand: one reference simulator rejects conversions in calls, the other gives these
-- results for the scalar formals but ignores both conversions of the composite formals.
entity call_conversions is end;
architecture t of call_conversions is
  type rgb is (red, green, blue);
  subtype byte is bit_vector(7 downto 0);
  type pt is record x, y : integer; end record;
  type pr is record a, b : real; end record;

  function to_r(i : integer) return real is begin return real(i); end function;
  function to_i(r : real) return integer is begin return integer(r); end function;
  function to_c(i : integer) return rgb is begin return rgb'val(i mod 3); end function;
  function to_n(c : rgb) return integer is begin return rgb'pos(c); end function;
  function rev(b : byte) return byte is
    variable r : byte;
  begin
    for k in 0 to 7 loop
      r(k) := b(7 - k);
    end loop;
    return r;
  end function;

  function to_pr(p : pt) return pr is begin return (real(p.x), real(p.y)); end function;
  function to_pt(p : pr) return pt is begin return (integer(p.a), integer(p.b)); end function;

  procedure seti(variable x : out integer) is begin x := 3; end procedure;
  procedure inc(variable x : inout integer) is begin x := x + 1; end procedure;
  procedure dbl(variable x : inout natural) is begin x := x * 2; end procedure;
  procedure shl(variable b : inout byte) is begin b := b(6 downto 0) & '1'; end procedure;
  procedure fill(variable b : out byte) is begin b := "00001111"; end procedure;
  procedure swap(variable p : inout pt) is begin p := (p.y, p.x + 10); end procedure;
  -- The actual of an in variable may be an actual conversion too.
  procedure show(variable x : in integer) is
  begin
    report "show " & integer'image(x);
  end procedure;
begin
  process
    variable r : real := 1.0;
    variable c : rgb := green;
    variable b : byte := "10000000";
    variable i : integer := 7;
    variable q : pr := (1.0, 2.0);
  begin
    seti(to_r(x) => r);
    assert r = 3.0;
    report "seti " & integer'image(integer(r));
    inc(to_r(x) => to_i(r));
    assert r = 4.0;
    report "inc " & integer'image(integer(r));
    inc(to_c(x) => to_n(c));
    report "rgb " & rgb'image(c);
    inc(x => i);
    dbl(to_r(x) => to_i(r));
    report "dbl " & integer'image(integer(r));
    shl(rev(b) => rev(b));
    report "shl " & to_string(b);
    fill(rev(b) => b);
    report "fill " & to_string(b);
    swap(to_pr(p) => to_pt(q));
    report "swap " & integer'image(integer(q.a)) & " " & integer'image(integer(q.b));
    show(to_i(r));
    show(to_n(c));
    wait;
  end process;
end architecture;
