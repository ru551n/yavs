library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_unsigned.all;
entity top is end;
architecture a of top is
  signal a, b : std_logic_vector(3 downto 0) := "0101";
begin
  process begin
    assert a = b;
    report "RES " & boolean'image(a = b);
    wait;
  end process;
end;
