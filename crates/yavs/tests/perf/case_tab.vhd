-- A hex-digit parser: a character to an integer and to a nibble through case functions of
-- 22 arms (the shape of an S-record reader). Result tables (#722) answer the calls inline, so
-- the memo of pure calls is not asked for them.
library ieee; use ieee.std_logic_1164.all;
entity case_tab is end;
architecture t of case_tab is
  function hexval(c : character) return integer is
  begin
    case c is
      when '0' => return 0; when '1' => return 1; when '2' => return 2; when '3' => return 3;
      when '4' => return 4; when '5' => return 5; when '6' => return 6; when '7' => return 7;
      when '8' => return 8; when '9' => return 9;
      when 'a' | 'A' => return 10; when 'b' | 'B' => return 11; when 'c' | 'C' => return 12;
      when 'd' | 'D' => return 13; when 'e' | 'E' => return 14; when 'f' | 'F' => return 15;
      when others => return -1;
    end case;
  end function;
  function nibble(c : character) return std_ulogic_vector is
    variable r : std_ulogic_vector(0 to 3);
  begin
    case c is
      when '0' => r := "0000"; when '1' => r := "0001"; when '2' => r := "0010";
      when '3' => r := "0011"; when '4' => r := "0100"; when '5' => r := "0101";
      when '6' => r := "0110"; when '7' => r := "0111"; when '8' => r := "1000";
      when '9' => r := "1001";
      when 'A' | 'a' => r := "1010"; when 'B' | 'b' => r := "1011";
      when 'C' | 'c' => r := "1100"; when 'D' | 'd' => r := "1101";
      when 'E' | 'e' => r := "1110"; when 'F' | 'f' => r := "1111";
      when others => r := "XXXX";
    end case;
    return r;
  end function;
begin
  process
    constant line : string := "S31500000000A0B1C2D3E4F5061728394A5B6C7D8E9F0011";
    variable v : std_ulogic_vector(0 to 3);
    variable sum, bits : integer := 0;
  begin
    for rep in 1 to 8 loop
      for i in line'range loop
        sum := sum + hexval(line(i));
        v := nibble(line(i));
        for j in 0 to 3 loop
          if v(j) = '1' then bits := bits + 1; end if;
        end loop;
      end loop;
    end loop;
    report "sum=" & integer'image(sum) & " bits=" & integer'image(bits);
    wait;
  end process;
end;
