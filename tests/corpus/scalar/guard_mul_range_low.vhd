-- difftest: top=guard_mul_range_low exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error.
-- A constant times a variable (the constant on the left) in the range of integer: compiled
-- code tests the variable against the range divided by the constant before it multiplies.
-- 7 * -306783378 is inside, 7 * -306783379 is outside on the fourth clock; the error is the
-- interpreter's overflow of the product.
entity guard_mul_range_low is end;

architecture a of guard_mul_range_low is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable x : integer := -306783376;
    variable r : integer := 0;
  begin
    if clk = '1' then
      r := 7 * x;
      report "x " & integer'image(x) & " r " & integer'image(r);
      x := x - 1;
    end if;
  end process;
end;
