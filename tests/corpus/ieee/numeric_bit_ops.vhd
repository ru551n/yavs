-- difftest: top=numeric_bit_ops std=2008
-- numeric_bit body: arithmetic, comparison, resize, conversions, shifts and division on bit vectors.
library ieee; use ieee.numeric_bit.all;
entity numeric_bit_ops is end;
architecture t of numeric_bit_ops is
begin
  process
    variable a : unsigned(7 downto 0) := to_unsigned(180, 8);
    variable b : unsigned(7 downto 0) := to_unsigned(13, 8);
    variable s : signed(7 downto 0) := to_signed(-77, 8);
    variable u : signed(7 downto 0) := to_signed(9, 8);
  begin
    report "add " & to_string(a + b) & " sub " & to_string(a - b) & " mul " & integer'image(to_integer(a * b));
    assert to_integer(a + b) = 193 and to_integer(a * b) = 2340 severity failure;
    report "div " & integer'image(to_integer(a / b)) & " rem " & integer'image(to_integer(a rem b)) & " mod " & integer'image(to_integer(a mod b));
    assert to_integer(a / b) = 13 and to_integer(a rem b) = 11 severity failure;
    report "sdiv " & integer'image(to_integer(s / u)) & " srem " & integer'image(to_integer(s rem u)) & " smod " & integer'image(to_integer(s mod u));
    assert to_integer(s / u) = -8 and to_integer(s rem u) = -5 and to_integer(s mod u) = 4 severity failure;
    report "abs " & to_string(abs s) & " neg " & to_string(-u) & " resize " & to_string(resize(s, 10));
    assert a > b and s < u and a /= 0 and s = -77 severity failure;
    report "shift " & to_string(shift_left(a, 1)) & " " & to_string(shift_right(s, 2)) & " " & to_string(rotate_left(b, 4)) & " " & to_string(a sra 1);
    assert shift_right(s, 2) = to_signed(-20, 8) severity failure;
    report "red " & bit'image(xor a) & " max " & to_string(maximum(a, b));
    report "done";
    wait;
  end process;
end;
