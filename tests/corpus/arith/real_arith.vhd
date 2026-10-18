-- difftest: top=real_arith std=2008
library ieee; use ieee.math_real.all;
entity real_arith is end;
architecture t of real_arith is
begin
  process
    variable x : real := 10.0;
  begin
    report "div " & integer'image(integer(x / 4.0 * 1000.0)) & " mul " & integer'image(integer(x * 0.1 * 1000.0));
    report "sqrt " & integer'image(integer(sqrt(16.0))) & " floor " & integer'image(integer(floor(2.7))) & " ceil " & integer'image(integer(ceil(2.1)));
    report "round " & integer'image(integer(round(2.5))) & " " & integer'image(integer(round(-2.5))) & " trunc " & integer'image(integer(trunc(-2.7)));
    report "pow " & integer'image(integer(2.0 ** 8)) & " " & integer'image(integer(x ** (-1) * 1000.0));
    assert abs(sin(MATH_PI / 2.0) - 1.0) < 1.0e-12 severity failure;
    assert abs(log(MATH_E) - 1.0) < 1.0e-12 and abs(exp(0.0) - 1.0) < 1.0e-12 severity failure;
    report "int " & integer'image(integer(2.5)) & " " & integer'image(integer(3.5)) & " " & integer'image(integer(-0.5));
    wait;
  end process;
end;
