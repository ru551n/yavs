-- difftest: top=numeric_std_93 std=1993
-- The 1993 NUMERIC_STD: UNSIGNED and SIGNED are arrays of STD_LOGIC.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity numeric_std_93 is end;

architecture a of numeric_std_93 is
begin
  process
    variable u, w : unsigned(7 downto 0);
    variable s : signed(7 downto 0);
    variable v : std_logic_vector(7 downto 0);
  begin
    u := to_unsigned(200, 8);
    w := to_unsigned(77, 8);
    report integer'image(to_integer(u + w));      -- wraps: 277 mod 256 = 21
    report integer'image(to_integer(u - w));
    report integer'image(to_integer(u * w));
    report boolean'image(u > w) & boolean'image(u = w) & boolean'image(u <= 200);
    s := to_signed(-100, 8);
    report integer'image(to_integer(s)) & " " & integer'image(to_integer(s + 30));
    report integer'image(to_integer(shift_left(w, 1))) & " " & integer'image(to_integer(shift_right(s, 2)));
    report integer'image(to_integer(resize(s, 12))) & " " & integer'image(to_integer(resize(u, 4)));
    v := std_logic_vector(u);
    assert v = "11001000" severity failure;
    w := unsigned(v) and "00001111";
    report integer'image(to_integer(w));
    report integer'image(to_integer(u / to_unsigned(7, 8))) & " " & integer'image(to_integer(u rem to_unsigned(7, 8)));
    assert u + 1 = 201 severity failure;
    wait;
  end process;
end architecture;
