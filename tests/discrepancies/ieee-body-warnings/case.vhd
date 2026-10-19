-- (from the differential corpus)
-- Warnings the numeric_std body reports itself (LRM source, NO_WARNING = false). ref=hand: sim1
-- runs numeric_std natively and prints none of them, even with --ieee-warnings=on; sim2 prints
-- the same three warnings from the IEEE source. Expected output hand-written from the source.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity ieee_warnings is end;
architecture t of ieee_warnings is
begin
  process
    variable u : unsigned(7 downto 0);
    variable n : integer;
  begin
    u := to_unsigned(300, 8);
    report "trunc " & to_string(u);
    n := to_integer(unsigned'("1X01"));
    report "meta " & integer'image(n);
    assert unsigned'("1X01") /= 13 severity failure;
    wait;
  end process;
end;
