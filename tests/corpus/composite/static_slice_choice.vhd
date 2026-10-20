-- difftest: top=tb_static_slice_choice std=2008
-- A slice of a locally static constant and a call of a numeric_std function with locally
-- static actuals are locally static, so they are legal case and selected assignment choices.
library ieee;
use ieee.numeric_std.all;
entity tb_static_slice_choice is end;
architecture t of tb_static_slice_choice is
  constant lo : integer := 0;
  constant hi : integer := 7;
  constant k1 : bit_vector(hi downto lo) := x"03";
  constant k2 : bit_vector(hi downto lo) := x"04";
  signal b : bit_vector(7 downto 0);
  signal s : unsigned(7 downto 0);
  signal o : bit;
begin
  with s select o <= '1' when to_unsigned(42, 8),
                     '0' when others;
  process (b) begin
    case b is
      when k1(7 downto 0) => report "k1";
      when k2(hi downto lo) => report "k2";
      when others => report "other";
    end case;
  end process;
  process begin
    wait for 1 ns;
    b <= x"03";
    s <= to_unsigned(42, 8);
    wait for 1 ns;
    b <= x"04";
    s <= to_unsigned(1, 8);
    wait for 1 ns;
    b <= x"05";
    report "o=" & bit'image(o);
    wait;
  end process;
end;
