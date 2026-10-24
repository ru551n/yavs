-- numeric_std operators on vectors called 100 times: the operands go to the native as values
-- (data address and bounds), no fat pair or descriptor is built for a freshly made vector
-- (the concatenation) and a parameter's descriptor is read three times.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity ieee_operands is end;
architecture a of ieee_operands is
  function sum(x, y : unsigned) return integer is
  begin
    return to_integer(x + y);
  end function;
begin
  process
    variable a : unsigned(7 downto 0) := x"5A";
    variable b : unsigned(7 downto 0) := x"C3";
    variable acc : integer := 0;
  begin
    for k in 1 to 100 loop
      a := a + 1;
      b := b + a;
      acc := (acc + sum(a, b) + to_integer(unsigned(a(3 downto 0) & b(3 downto 0)))) mod 100000;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
