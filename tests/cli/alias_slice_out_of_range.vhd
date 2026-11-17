-- An alias of a slice outside its prefix's range is an error when the alias is elaborated, read
-- or not (LRM 6.6.2, 8.5); analysis only warns (W1404), since the code may not be elaborated.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity alias_slice_out_of_range is
end entity;
architecture a of alias_slice_out_of_range is
  signal i6 : signed(0 downto 0) := "1";
  alias al : signed(0 to 18) is i6(32 downto 14);
begin
  process
  begin
    report "len " & integer'image(al'length);
    wait;
  end process;
end architecture;
