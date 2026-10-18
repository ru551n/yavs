-- difftest: top=int_ops std=2008
entity int_ops is end;
architecture t of int_ops is
begin
  process
    variable a : integer := 17; variable b : integer := -5;
  begin
    report "div " & integer'image(a / b) & " " & integer'image(b / 2);
    report "mod " & integer'image(a mod b) & " " & integer'image(b mod 3) & " " & integer'image(a mod 5);
    report "rem " & integer'image(a rem b) & " " & integer'image(b rem 3);
    report "abs " & integer'image(abs b) & " exp " & integer'image(2 ** 10) & " " & integer'image((-2) ** 3);
    report "neg " & integer'image(-a) & " mul " & integer'image(a * b);
    assert (a mod b) = -3 and ((-17) mod 5) = 3 and ((-17) rem 5) = -2 severity failure;
    assert integer'image(integer'low + 1) = "-2147483647" severity failure;
    report "minmax " & integer'image(maximum(a, b)) & " " & integer'image(minimum(a, b));
    wait;
  end process;
end;
