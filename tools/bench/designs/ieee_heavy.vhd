-- numeric_std and std_logic_1164 on vectors as scoreboards and bus models use them: conversions,
-- arithmetic, comparison, resizing, logical operators and slicing.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity bench_ieee_heavy is
  generic (N : natural := 100000);
end entity;

architecture sim of bench_ieee_heavy is
begin
  process
    variable a, b, c : unsigned(31 downto 0);
    variable s       : signed(15 downto 0);
    variable v, w    : std_logic_vector(31 downto 0);
    variable sum     : natural := 0;
  begin
    for i in 1 to N loop
      a := to_unsigned(i, 32);
      b := a + 3;
      c := b - a + resize(to_unsigned(i mod 251, 8), 32);
      s := to_signed(i mod 1000 - 500, 16);
      v := std_logic_vector(c) and x"0000FFFF";
      w := v xor std_logic_vector(a);
      if a < b and unsigned(w) /= 0 then
        sum := (sum + to_integer(unsigned(v(7 downto 0)))) mod 1000003;
      end if;
      sum := (sum + to_integer(s) + 500 + to_integer(shift_left(a, 2) srl 1)) mod 1000003;
      if w(0) = '1' and (w(31 downto 28) = "0000" or w(1) /= '0') then
        sum := sum + 1;
      end if;
    end loop;
    report "ieee_heavy sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
