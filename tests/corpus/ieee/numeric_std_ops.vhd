-- difftest: top=numeric_std_ops std=2008
-- numeric_std body: arithmetic, comparison, resize, conversions, shifts, division, std_match.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity numeric_std_ops is end;
architecture t of numeric_std_ops is
begin
  process
    variable a : unsigned(7 downto 0) := to_unsigned(200, 8);
    variable b : unsigned(7 downto 0) := to_unsigned(57, 8);
    variable s : signed(7 downto 0) := to_signed(-100, 8);
    variable u : signed(7 downto 0) := to_signed(37, 8);
    variable acc : unsigned(15 downto 0) := (others => '0');
    variable q : integer;
  begin
    report "add " & to_string(a + b) & " sub " & to_string(a - b) & " mul " & to_string(a * b);
    assert to_integer(a + b) = 1 and to_integer(a - b) = 143 and to_integer(a * b) = 11400 severity failure;
    report "div " & integer'image(to_integer(a / b)) & " rem " & integer'image(to_integer(a rem b)) & " mod " & integer'image(to_integer(a mod b));
    assert to_integer(a / b) = 3 and to_integer(a rem b) = 29 severity failure;
    report "sdiv " & integer'image(to_integer(s / u)) & " srem " & integer'image(to_integer(s rem u)) & " smod " & integer'image(to_integer(s mod u));
    assert to_integer(s / u) = -2 and to_integer(s rem u) = -26 and to_integer(s mod u) = 11 severity failure;
    report "sadd " & to_string(s + u) & " smul " & integer'image(to_integer(s * u)) & " abs " & to_string(abs s) & " neg " & to_string(-u);
    assert to_integer(s * u) = -3700 severity failure;
    assert a > b and b < a and s < u and a >= 200 and s = -100 and not (a = b) severity failure;
    assert maximum(a, b) = a and minimum(s, u) = s severity failure;
    report "resize " & to_string(resize(s, 12)) & " " & to_string(resize(a, 4)) & " " & to_string(resize(u, 4));
    assert to_integer(resize(s, 12)) = -100 severity failure;
    report "shift " & to_string(shift_left(a, 2)) & " " & to_string(shift_right(s, 3)) & " " & to_string(rotate_left(b, 3)) & " " & to_string(rotate_right(b, 1));
    assert shift_right(s, 3) = to_signed(-13, 8) and (a srl 4) = 12 severity failure;
    for i in 1 to 50 loop acc := acc + i * i; end loop;
    report "sum of squares " & integer'image(to_integer(acc));
    assert to_integer(acc) = 42925 severity failure;
    report "slv " & to_string(std_logic_vector(a)) & " back " & integer'image(to_integer(unsigned(std_logic_vector(b))));
    report "find " & integer'image(find_leftmost(a, '1')) & " " & integer'image(find_rightmost(a, '1'));
    assert std_match(unsigned'("10-1"), unsigned'("1011")) severity failure;
    assert ("1010" ?= unsigned'("10-0")) = '1' severity failure;
    assert to_01(unsigned'("1X0Z"), '1') = "1111" and to_01(unsigned'("1H0L")) = "1100" severity failure;
    q := to_integer(unsigned'(x"FFFF")) + to_integer(signed'(x"8000"));
    report "q " & integer'image(q);
    assert q = 32767 severity failure;
    report "and " & to_string(a and b) & " xor " & to_string(s xor u) & " red " & std_ulogic'image(and a);
    report "done";
    wait;
  end process;
end;
