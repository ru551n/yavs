-- difftest: top=case_forms std=2008
library ieee;
use ieee.std_logic_1164.all;
entity case_forms is end;
architecture t of case_forms is
  subtype word is string(1 to 4);
  function name(s : word) return integer is
  begin
    case s is
      when "red " => return 1;
      when "blue" => return 2;
      when others => return 0;
    end case;
  end;
  function dec(v : std_ulogic_vector(2 downto 0)) return integer is
  begin
    case? v is
      when "1--" => return 4;
      when "01-" => return 2;
      when "001" => return 1;
      when others => return 0;
    end case?;
  end;
  signal sel : bit_vector(1 downto 0) := "00";
  signal y : integer := 0;
begin
  with sel select y <= 10 when "00", 11 when "01", 12 when "10", 13 when others;
  process
  begin
    report integer'image(name("red ")) & integer'image(name("blue")) & integer'image(name("xxxx"));
    report integer'image(dec("1X0")) & integer'image(dec("011")) & integer'image(dec("001"))
      & integer'image(dec("0H1")) & integer'image(dec("000"));
    for i in 0 to 3 loop
      case i is when 0 => sel <= "00"; when 1 => sel <= "01"; when 2 => sel <= "10"; when others => sel <= "11"; end case;
      wait for 1 ns;
      report "y=" & integer'image(y);
    end loop;
    wait;
  end process;
end;
