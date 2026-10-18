-- difftest: top=conditional_expr_2019 std=2019
-- 2019 conditional expressions (9.1) in initial values, assignments, actuals and
-- `return` (10.14: a conditional return goes on when no condition holds). The
-- parenthesized primary form is left out: the reference simulator does not accept
-- it yet (VI#385). `unaffected` is in unaffected_2019.
entity conditional_expr_2019 is end;
architecture t of conditional_expr_2019 is
  constant g : boolean := true;
  constant k : integer := 10 when not g else 20;
  signal s : bit_vector(3 downto 0) := (others => '1') when g else "0000";

  function sign(x : integer) return integer is
  begin
    return 1 when x > 0 else -1 when x < 0;
    report "sign of zero";
    return 0;
  end function;

  function pick(b : boolean; x, y : string) return string is
  begin
    return x when b else y;
  end function;

  function plus(a, b : integer) return integer is
  begin
    return a + b;
  end function;
begin
  process
    variable v : integer := 4 when g else 2;
    variable w : bit_vector(3 downto 0) := "0101";
  begin
    assert k = 20 severity failure;
    assert s = "1111" severity failure;
    assert v = 4 severity failure;
    report "sign " & integer'image(sign(5)) & " " & integer'image(sign(-5)) & " " & integer'image(sign(0));
    report pick(true, "yes", "no") & " " & pick(false, "yes", "no!");
    v := plus(1 when v > 3 else 2, 10 when v < 3 else 20);
    report "plus " & integer'image(v);
    v := 6 when v = 0;
    report "no else " & integer'image(v);
    v := 7 when v = 0 else 8 when v = 21 else 9;
    report "chain " & integer'image(v);
    w := (others => '0') when w(0) = '1' else "1111";
    assert w = "0000" severity failure;
    wait;
  end process;
end;
