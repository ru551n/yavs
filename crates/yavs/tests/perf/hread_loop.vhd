-- The digit loop of a textio HRead: a hex digit to a nibble through a case function (a result
-- table, #722), the nibble stored in a slice `sv(4*i to 4*i+3)` of the result and tested with
-- `is_x` on the same slice (#744). The slices have a constant length 4 whatever `i` is, so the
-- length checks and the descriptors of the slices are folded, the pair the table read builds
-- is not made, and the second slice position is the first one again.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity hread_loop is end;
architecture t of hread_loop is
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

  procedure hex(s : in string; value : out std_ulogic_vector; good : out boolean) is
    constant sl : integer := value'length;
    variable sv : std_ulogic_vector(0 to sl - 1);
  begin
    for i in 0 to sl / 4 - 1 loop
      sv(4 * i to 4 * i + 3) := nibble(s(s'left + i));
      if is_x(sv(4 * i to 4 * i + 3)) then
        good := false;
        sv := (others => 'X');
        value := sv;
        return;
      end if;
    end loop;
    good := true;
    value := sv;
  end procedure;
begin
  process
    constant text : string := "0123abCD89ef4567DEADBEEF";
    variable v : std_ulogic_vector(31 downto 0);
    variable g : boolean;
    variable sum : natural := 0;
  begin
    for rep in 1 to 16 loop
      for k in 0 to 2 loop
        hex(text(1 + 8 * k to 8 + 8 * k), v, g);
        assert g;
        sum := (sum + to_integer(unsigned(v(15 downto 0)))) mod 65521;
      end loop;
    end loop;
    hex("12G4", v(15 downto 0), g);
    assert not g and is_x(v(15 downto 0)) report "a bad digit must give X" severity failure;
    assert sum = 965 report "bad sum" severity failure;
    report "sum=" & integer'image(sum);
    wait;
  end process;
end;
