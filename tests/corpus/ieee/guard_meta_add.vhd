-- difftest: top=guard_meta_add std=2008 ref=hand
-- ref=hand: the expected text is yavs's wording of the warning of numeric_std.
-- An unsigned addition in compiled code whose operand holds a metavalue from the third clock
-- on: the test of the operands' planes sends it to the native, which warns at that time and
-- returns all 'X'; on the fifth clock the metavalue is gone again.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity guard_meta_add is end;

architecture a of guard_meta_add is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 60 ns;
  process (clk)
    variable a : unsigned(7 downto 0) := x"10";
    variable b : unsigned(7 downto 0) := x"01";
    variable n : integer := 0;
  begin
    if clk = '1' then
      n := n + 1;
      if n = 3 then
        b(2) := 'X';
      elsif n = 5 then
        b(2) := '0';
        a := x"20";
      end if;
      a := b + a;
      report "n " & integer'image(n) & " a " & to_hstring(a);
    end if;
  end process;
end;
