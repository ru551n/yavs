-- A 65 by 65 element product, a 128-element sum, reductions of 65 elements and concatenations of
-- 65 and 128 elements: compiled code computes each in place, no native callback and no step.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity wide_ieee is end;
architecture a of wide_ieee is
begin
  process
    variable seed : unsigned(63 downto 0) := x"0123456789abcdef";
    variable x, y : std_ulogic_vector(63 downto 0);
    variable a, b : std_ulogic_vector(64 downto 0);
    variable p : std_ulogic_vector(129 downto 0);
    variable d : std_ulogic_vector(127 downto 0);
    variable n : natural := 0;
  begin
    for k in 1 to 200 loop
      seed := seed(62 downto 0) & (seed(63) xor seed(62) xor seed(60) xor seed(59));
      x := std_ulogic_vector(seed);
      y := std_ulogic_vector(seed rol 23);
      a := (x(63) and y(1)) & x;
      b := y(63) & y;
      p := std_ulogic_vector(signed(a) * signed(b));
      d := std_ulogic_vector(unsigned(x & y) + unsigned(p(127 downto 0)));
      n := (n * 3 + to_integer(unsigned(p(129 downto 118))) + to_integer(unsigned(d(127 downto 116)))
            + std_ulogic'pos(or d(127 downto 63)) + std_ulogic'pos(and a))
           mod 99991;
      wait for 1 ns;
    end loop;
    report "n=" & integer'image(n);
    wait;
  end process;
end;
