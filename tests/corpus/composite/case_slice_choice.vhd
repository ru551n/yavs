-- difftest: top=case_slice_choice std=2008
-- A slice of a locally static constant with a locally static range is a locally static case
-- choice (LRM 9.4.2), also with a subtype as the range and in a matching case.
library ieee;
use ieee.std_logic_1164.all;
entity case_slice_choice is end entity;
architecture a of case_slice_choice is
  subtype hi_range is natural range 7 downto 4;
  constant k : bit_vector(7 downto 0) := X"A5";
  constant m : std_ulogic_vector(7 downto 0) := "10--0101";
  signal s : bit_vector(3 downto 0) := "0000";
  signal u : std_ulogic_vector(7 downto 0) := "10110101";
begin
  process
    procedure test (v : bit_vector(3 downto 0)) is
    begin
      case v is
        when k(7 downto 4) => report "high nibble";
        when k(hi_range'low - 1 downto 0) => report "low nibble";
        when k(4 downto 4) & "000" => report "bit 4";
        when others => report "other";
      end case;
    end procedure;
  begin
    test(x"A");
    test(x"5");
    test(x"3");
    case? u is
      when m(7 downto 0) => report "match";
      when others => report "no match";
    end case?;
    wait;
  end process;
end architecture;
