-- difftest: top=return_when_2019 std=2019
-- 2019 `return when c;` (10.14): a procedure returns when the condition holds and goes on
-- otherwise.
entity return_when_2019 is end;
architecture t of return_when_2019 is
  procedure clip(x : inout integer; lim : integer) is
  begin
    return when x <= lim;
    report "clip " & to_string(x) & " to " & to_string(lim);
    x := lim;
  end procedure;
begin
  process
    variable v : integer := 3;
  begin
    clip(v, 5);
    assert v = 3 severity failure;
    v := 9;
    clip(v, 5);
    assert v = 5 severity failure;
    report "v " & to_string(v);
    wait;
  end process;
end;
