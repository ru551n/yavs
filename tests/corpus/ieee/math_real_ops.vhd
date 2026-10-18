-- difftest: top=math_real_ops std=2008
-- math_real body: rounding, roots, exp/log, trig, hyperbolic, uniform. Values are printed as
-- integers scaled by 1e6 (real'image text is implementation-defined, see the README).
library ieee; use ieee.math_real.all;
entity math_real_ops is end;
architecture t of math_real_ops is
  function scaled(x : real) return integer is begin return integer(round(x * 1.0e6)); end;
  procedure check(name : string; got, want : real) is
  begin
    report name & " " & integer'image(scaled(got));
    assert abs (got - want) <= 1.0e-6 * (1.0 + abs want) report name & " mismatch" severity failure;
  end;
begin
  process
    variable s1 : positive := 42;
    variable s2 : positive := 7;
    variable x : real;
  begin
    check("ceil", ceil(2.3), 3.0); check("floor", floor(-2.3), -3.0);
    check("round", round(2.5), 3.0); check("round_neg", round(-2.5), -3.0);
    check("trunc", trunc(-2.7), -2.0); check("sign", sign(-4.0), -1.0);
    check("mod", 7.5 mod 2.0, 1.5); check("realmax", realmax(1.0, 2.0), 2.0);
    check("sqrt", sqrt(2.0), 1.4142135623730951); check("cbrt", cbrt(27.0), 3.0);
    check("pow", 2.0 ** 0.5, 1.4142135623730951); check("ipow", 3 ** 2.0, 9.0);
    check("exp", exp(1.0), MATH_E); check("log", log(MATH_E), 1.0);
    check("log2", log2(8.0), 3.0); check("log10", log10(1000.0), 3.0); check("logb", log(81.0, 3.0), 4.0);
    check("sin", sin(MATH_PI / 6.0), 0.5); check("cos", cos(MATH_PI / 3.0), 0.5);
    check("tan", tan(MATH_PI / 4.0), 1.0); check("arcsin", arcsin(1.0), MATH_PI_OVER_2);
    check("arccos", arccos(0.0), MATH_PI_OVER_2); check("arctan", arctan(1.0), MATH_PI / 4.0);
    check("arctan2", arctan(-1.0, -1.0), -3.0 * MATH_PI / 4.0);
    check("sinh", sinh(1.0), 1.1752011936438014); check("cosh", cosh(1.0), 1.5430806348152437);
    check("tanh", tanh(0.5), 0.46211715726000974); check("arcsinh", arcsinh(1.0), 0.881373587019543);
    check("arccosh", arccosh(2.0), 1.3169578969248166); check("arctanh", arctanh(0.5), 0.5493061443340549);
    for i in 1 to 1000 loop
      uniform(s1, s2, x);
      assert x > 0.0 and x < 1.0 severity failure;
    end loop;
    report "uniform " & integer'image(scaled(x)) & " seeds " & integer'image(s1) & " " & integer'image(s2);
    report "done";
    wait;
  end process;
end;
