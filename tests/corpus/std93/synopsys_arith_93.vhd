-- difftest: top=synopsys_arith_93 std=1993 relaxed
-- The Synopsys packages on 1993 STD_LOGIC_VECTOR: "=" of std_logic_1164 and of std_logic_unsigned are
-- two visible homographs, the explicit one wins (the references need their relaxed switch for it).
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_unsigned.all;
use ieee.std_logic_arith.all;

entity synopsys_arith_93 is end;

architecture a of synopsys_arith_93 is
begin
  process
    variable a, b : std_logic_vector(7 downto 0);
    variable u : unsigned(7 downto 0);
  begin
    a := conv_std_logic_vector(100, 8);
    b := a + 1;
    report integer'image(conv_integer(b));
    assert a < b severity failure;
    assert a = conv_std_logic_vector(100, 8) severity failure;
    u := conv_unsigned(5, 8) + unsigned(a);
    report integer'image(conv_integer(u));
    b := a + "00000011";
    report integer'image(conv_integer(b)) & " " & integer'image(conv_integer(a(3 downto 0)));
    wait;
  end process;
end architecture;
