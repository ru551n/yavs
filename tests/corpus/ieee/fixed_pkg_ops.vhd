-- difftest: top=fixed_pkg_ops std=2008
-- fixed_pkg (instance of fixed_generic_pkg): conversions, arithmetic with growth, resize with
-- rounding and saturation, division, comparison, strings.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use ieee.fixed_pkg.all; use ieee.fixed_float_types.all;
entity fixed_pkg_ops is end;
architecture t of fixed_pkg_ops is
begin
  process
    variable a : sfixed(3 downto -4) := to_sfixed(1.5, 3, -4);
    variable b : sfixed(3 downto -4) := to_sfixed(-2.25, 3, -4);
    variable c : ufixed(4 downto -3) := to_ufixed(6.375, 4, -3);
    variable d : ufixed(2 downto -2) := to_ufixed(1.75, 2, -2);
    variable r : sfixed(3 downto -2);
  begin
    report "a " & to_string(a) & " b " & to_string(b) & " c " & to_string(c);
    report "add " & to_string(a + b) & " sub " & to_string(a - b) & " mul " & to_string(a * b);
    assert to_real(a + b) = -0.75 and to_real(a * b) = -3.375 severity failure;
    report "uadd " & to_string(c + d) & " umul " & to_string(c * d);
    assert to_real(c * d) = 11.15625 severity failure;
    report "div " & to_string(c / d) & " real " & integer'image(integer(to_real(c / d) * 1000.0));
    report "neg " & to_string(-a) & " abs " & to_string(abs b);
    r := resize(to_sfixed(2.875, 3, -4), 3, -2);
    report "round " & to_string(r);
    assert to_real(r) = 3.0 severity failure;
    r := resize(to_sfixed(2.875, 3, -4), 3, -2, fixed_saturate, fixed_truncate);
    assert to_real(r) = 2.75 severity failure;
    r := resize(to_sfixed(9.5, 4, -4), 3, -2);
    report "saturate " & to_string(r);
    assert to_real(r) = 7.75 severity failure;
    assert a > b and b < a and a = to_sfixed(1.5, 3, -4) and c >= d severity failure;
    report "int " & integer'image(to_integer(to_sfixed(-5, 4, 0))) & " slv " & to_string(to_slv(a));
    assert to_integer(c) = 6 severity failure;
    report "hstr " & to_hstring(c) & " ostr " & to_ostring(a);
    report "from_str " & to_string(sfixed'(from_string("0110.1000", 3, -4)));
    assert to_real(sfixed'(from_string("0110.1000", 3, -4))) = 6.5 severity failure;
    report "unsigned " & to_string(to_ufixed(unsigned'("1011"), 3, 0)) & " scalb " & to_string(scalb(a, 1));
    report "done";
    wait;
  end process;
end;
