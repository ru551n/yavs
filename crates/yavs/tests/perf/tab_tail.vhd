-- A result-table read that branches (#769): the argument is of an integer type, so the read
-- tests its range and a validity mask (the digits 16 to 20 report, so they are not tabulated).
-- The rest of the block is copied into the arm that reads the table, where the loads of the
-- array result find the stores that built it: no pair is made, as for a character argument.
library ieee; use ieee.std_logic_1164.all;
entity tab_tail is end;
architecture t of tab_tail is
  type dig_t is range 0 to 20;
  type da is array (0 to 7) of dig_t;
  function nibble(n : dig_t) return std_ulogic_vector is
    variable r : std_ulogic_vector(0 to 3);
  begin
    case n is
      when 0 => r := "0000"; when 1 => r := "0001"; when 2 => r := "0010";
      when 3 => r := "0011"; when 4 => r := "0100"; when 5 => r := "0101";
      when 6 => r := "0110"; when 7 => r := "0111"; when 8 => r := "1000";
      when 9 => r := "1001"; when 10 => r := "1010"; when 11 => r := "1011";
      when 12 => r := "1100"; when 13 => r := "1101"; when 14 => r := "1110";
      when 15 => r := "1111";
      when others => assert false report "bad digit" severity warning; r := "XXXX";
    end case;
    return r;
  end function;
begin
  process
    variable sv : std_ulogic_vector(0 to 31);
    variable sum : natural := 0;
    variable a : da := (0, 1, 2, 3, 4, 5, 6, 7);
  begin
    for rep in 1 to 100 loop
      for i in 0 to 7 loop
        sv(4 * i to 4 * i + 3) := nibble(a(i));
      end loop;
      if sv(7) = '1' then sum := sum + 1; end if;
      a(rep mod 8) := dig_t((rep * 3) mod 16);
    end loop;
    sv(0 to 3) := nibble(20);
    assert sv(0 to 3) = "XXXX" report "a digit that is no table entry must call" severity failure;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end;
