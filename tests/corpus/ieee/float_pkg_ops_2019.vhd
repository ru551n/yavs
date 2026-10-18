-- difftest: top=float_pkg_ops_2019 std=2019
-- float_pkg (instance of float_generic_pkg): conversions, arithmetic, rounding, classes,
-- comparison, special values, strings, a small accumulation.
library ieee; use ieee.std_logic_1164.all; use ieee.float_pkg.all;
entity float_pkg_ops_2019 is end;
architecture t of float_pkg_ops_2019 is
begin
  process
    variable a : float32 := to_float(1.5);
    variable b : float32 := to_float(-2.25);
    variable h : float(5 downto -10);
    variable acc : float32 := to_float(0.0);
  begin
    report "a " & to_string(a) & " b " & to_string(b);
    report "add " & to_string(a + b) & " sub " & to_string(a - b);
    report "mul " & to_string(a * b) & " div " & to_string(a / b);
    assert to_real(a + b) = -0.75 and to_real(a * b) = -3.375 and to_real(a - b) = 3.75 severity failure;
    report "neg " & to_string(-a) & " abs " & to_string(abs b);
    report "sqrt " & to_string(sqrt(to_float(2.0))) & " rem " & to_string(to_float(7.0) rem to_float(2.5));
    assert to_real(sqrt(to_float(16.0))) = 4.0 severity failure;
    report "third " & to_string(to_float(1.0) / to_float(3.0));
    assert a > b and b < a and a = to_float(1.5) and a /= b severity failure;
    report "int " & integer'image(to_integer(to_float(-7.6))) & " " & integer'image(to_integer(to_float(2.5)));
    assert to_integer(to_float(100)) = 100 severity failure;
    h := resize(to_float(3.14159), 5, 10);
    report "half " & to_string(h);
    report "class " & valid_fpstate'image(classfp(a)) & " " & valid_fpstate'image(classfp(to_float(0.0))) & " " & valid_fpstate'image(classfp(from_string("1_11111111_00000000000000000000000")));
    assert isnan(from_string("0_11111111_10000000000000000000000")) and not isnan(a) severity failure;
    assert classfp(from_string("0_11111111_00000000000000000000000")) = pos_inf severity failure;
    for i in 1 to 10 loop acc := acc + to_float(real(i) * 0.1); end loop;
    report "acc " & to_string(acc);
    report "slv " & to_string(to_slv(b)) & " hex " & to_hstring(a);
    report "from_str " & to_string(from_string("0_10000000_10000000000000000000000"));
    assert to_real(from_string("0_10000000_10000000000000000000000")) = 3.0 severity failure;
    report "done";
    wait;
  end process;
end;
