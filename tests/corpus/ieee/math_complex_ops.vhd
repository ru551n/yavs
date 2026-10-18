-- difftest: top=math_complex_ops std=2008
-- math_complex body: arithmetic, polar conversion, abs, arg, sqrt, exp, log, trig. Values are
-- printed scaled by 1e6 as integers.
library ieee; use ieee.math_real.all; use ieee.math_complex.all;
entity math_complex_ops is end;
architecture t of math_complex_ops is
  function scaled(x : real) return integer is begin return integer(round(x * 1.0e6)); end;
  procedure check(name : string; got, want : complex) is
  begin
    report name & " " & integer'image(scaled(got.re)) & " " & integer'image(scaled(got.im));
    assert abs (got.re - want.re) <= 1.0e-6 and abs (got.im - want.im) <= 1.0e-6
      report name & " mismatch" severity failure;
  end;
begin
  process
    constant a : complex := (3.0, 4.0);
    constant b : complex := (1.0, -2.0);
    variable p : complex_polar;
  begin
    check("add", a + b, (4.0, 2.0)); check("sub", a - b, (2.0, 6.0));
    check("mul", a * b, (11.0, -2.0)); check("div", a / b, (-1.0, 2.0));
    check("neg", -a, (-3.0, -4.0)); check("conj", conj(a), (3.0, -4.0));
    check("mixed", a + 1.0, (4.0, 4.0)); check("scale", 2.0 * b, (2.0, -4.0));
    assert abs(a) = 5.0 severity failure;
    report "abs " & integer'image(scaled(abs(a))) & " arg " & integer'image(scaled(arg(a)));
    p := complex_to_polar(a);
    report "polar " & integer'image(scaled(p.mag)) & " " & integer'image(scaled(p.arg));
    check("back", polar_to_complex(p), a);
    check("sqrt", sqrt((-4.0, 0.0)), (0.0, 2.0));
    check("exp", exp((0.0, MATH_PI)), (-1.0, 0.0));
    check("log", log((MATH_E, 0.0)), (1.0, 0.0));
    check("sin", sin((MATH_PI_OVER_2, 0.0)), (1.0, 0.0));
    check("cos", cos((0.0, 0.0)), (1.0, 0.0));
    report "done";
    wait;
  end process;
end;
