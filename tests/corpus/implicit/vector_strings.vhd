-- difftest: top=vector_strings std=2008
-- TO_STRING of std_ulogic_vector, std_logic_vector and bit_vector (LRM 2008 5.3.2.4) at lengths
-- around the 64-element blocks of their planes, every std_ulogic value, both directions, against
-- strings spelled element by element.
library ieee;
use ieee.std_logic_1164.all;

entity vector_strings is end entity;

architecture test of vector_strings is
  constant LENS : integer_vector := (0, 1, 2, 63, 64, 65, 127, 128, 129, 200);

  function spell(v : std_ulogic_vector) return string is
    variable s : string(1 to v'length);
    variable k : positive := 1;
  begin
    for i in v'range loop
      s(k) := std_ulogic'image(v(i))(2);
      k := k + 1;
    end loop;
    return s;
  end function;

  function spell(v : bit_vector) return string is
    variable s : string(1 to v'length);
    variable k : positive := 1;
  begin
    for i in v'range loop
      s(k) := bit'image(v(i))(2);
      k := k + 1;
    end loop;
    return s;
  end function;

  function from_one(s : string; n : natural) return boolean is
  begin
    return s'length = n and s'left = 1;
  end function;

  procedure one(n, pat : natural; sum : inout natural) is
    variable d : std_ulogic_vector(n - 1 downto 0);
    variable u : std_logic_vector(1 to n);
    variable b : bit_vector(0 to n - 1);
  begin
    for i in 0 to n - 1 loop
      case pat is
        -- every value
        when 0 => d(i) := std_ulogic'val((i * 5 + n) mod 9);
        -- '0' and '1' only
        when 1 => d(i) := '1' when (i * 7 + n) mod 3 = 0 else '0';
        -- '0' and '1' with one metavalue in the middle
        when others => d(i) := 'Z' when i = n / 2 else '1' when i mod 2 = 0 else '0';
      end case;
      u(i + 1) := d(i);
      b(i) := '1' when (i * 3 + pat) mod 4 = 1 else '0';
    end loop;
    assert to_string(d) = spell(d) severity failure;
    assert to_string(u) = spell(u) severity failure;
    assert to_string(b) = spell(b) severity failure;
    assert from_one(to_string(d), n) severity failure;
    if n <= 65 then
      report integer'image(n) & " " & to_string(d) & " " & to_string(u) & " " & to_string(b);
    end if;
    sum := sum + n;
  end procedure;
begin
  process
    variable sum : natural := 0;
  begin
    for t in LENS'range loop
      for pat in 0 to 2 loop
        one(LENS(t), pat, sum);
      end loop;
    end loop;
    report "done " & integer'image(sum);
    wait;
  end process;
end architecture;
