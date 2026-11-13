-- difftest: top=real_array_order_2019 std=2019
-- #1144: ordering of arrays of reals (VHDL-2019) is lexicographic on the values: -0.0 = 0.0 and
-- negative values order as numbers. Only sim1 runs it; its output is hand-checked (LRM 2019 9.2.3).
entity real_array_order_2019 is end;
architecture t of real_array_order_2019 is
  procedure show (name : string; a, b : real_vector) is
  begin
    report name & " lt " & boolean'image(a < b) & " le " & boolean'image(a <= b)
      & " gt " & boolean'image(a > b) & " ge " & boolean'image(a >= b)
      & " eq " & boolean'image(a = b);
  end procedure;
begin
  process
    variable z : real := 0.0;
    variable n1 : real_vector(1 to 0);
  begin
    z := -z;  -- negative zero
    show("zeros", (z, 5.0), (0.0, 1.0));
    show("zeros2", (0.0, 1.0), (z, 5.0));
    show("zeros3", (z, 1.0), (0.0, 1.0));
    show("neg", (-2.0, 1.0), (-1.0, 1.0));
    show("negpos", (-1.0, 1.0), (1.0, 1.0));
    show("pos", (1.0, 2.0), (1.0, 3.0));
    show("prefix", (1.0, 2.0), (1.0, 2.0, 0.0));
    show("longer", (1.0, 2.0, 0.0), (1.0, 2.0));
    show("null", n1, (0 => 1.0));
    show("first", (3.0, -9.0), (2.0, 9.0));
    wait;
  end process;
end;
