-- difftest: top=guard_pos_const std=2008 exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the run-time error.
-- An index into an array constant (its bounds known when the code is made) that leaves the
-- range on the fourth clock: compiled code tests the position and leaves, the interpreter
-- reports at that time. The position's interval drops the overflow tests of the element
-- offset computed from it, not the index test.
library ieee;
use ieee.std_logic_1164.all;

entity guard_pos_const is end;

architecture a of guard_pos_const is
  type bytes is array (2 to 9) of std_logic_vector(7 downto 0);
  constant tab : bytes := (x"01", x"02", x"04", x"08", x"10", x"20", x"40", x"80");
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable i : integer := 5;
    variable acc : std_logic_vector(7 downto 0) := (others => '0');
  begin
    if clk = '1' then
      acc := acc xor tab(i);
      report "i " & integer'image(i) & " acc " & to_hstring(acc);
      i := i + 2;
    end if;
  end process;
end;
