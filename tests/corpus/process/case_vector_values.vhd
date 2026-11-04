-- difftest: top=case_vector_values
-- `case` on logic vectors: values with metavalues never take a choice of '0' and '1', a choice
-- that holds a metavalue takes exactly that value, both directions, BIT_VECTOR, slices, named
-- constants, sizes up to 64 elements, an others-less case and a signal, variable and function value.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;

entity case_vector_values is
end entity;

architecture a of case_vector_values is
  constant K : std_ulogic_vector(3 downto 0) := "0110";
  subtype w63_t is std_logic_vector(62 downto 0);
  subtype w64_t is std_logic_vector(63 downto 0);
  constant TOP63 : w63_t := (62 => '1', others => '0');
  constant LOW63 : w63_t := (0 => '1', others => '0');
  constant ONES63 : w63_t := (others => '1');
  constant TOP64 : w64_t := (63 => '1', others => '0');
  constant ONES64 : w64_t := (others => '1');
  signal s : std_ulogic_vector(3 downto 0) := "UUUU";
  signal u : std_ulogic_vector(0 to 3) := "0000";
  signal n : unsigned(7 downto 0) := (others => '0');

  function f (x : std_ulogic_vector) return integer is
  begin
    case x is
      when "0000" => return 0;
      when "0001" | "0010" => return 1;
      when K => return 2;
      when "1111" => return 3;
      when "XXXX" => return 4;
      when "UU01" => return 5;
      when "--01" => return 6;
      when others => return 7;
    end case;
  end function;

  function g (x : w63_t) return integer is
  begin
    case x is
      when TOP63 => return 1;
      when LOW63 => return 2;
      when ONES63 => return 3;
      when others => return 4;
    end case;
  end function;

  function g64 (x : w64_t) return integer is
  begin
    case x is
      when TOP64 => return 1;
      when ONES64 => return 3;
      when others => return 4;
    end case;
  end function;

  function b (x : bit_vector(0 to 2)) return integer is
  begin
    case x is
      when "001" => return 1;
      when "100" => return 2;
      when "000" | "111" => return 3;
      when others => return 4;
    end case;
  end function;

  function up (x : std_ulogic_vector(0 to 3)) return integer is
  begin
    case x is
      when "0001" => return 1;
      when "1000" => return 8;
      when "HLHL" => return 9;
      when others => return 0;
    end case;
  end function;

  function total (x : bit_vector(1 downto 0)) return integer is
  begin
    case x is
      when "00" => return 0;
      when "01" => return 1;
      when "10" => return 2;
      when "11" => return 3;
    end case;
  end function;

  function img (x : std_ulogic_vector) return string is
    variable r : string(1 to x'length);
    variable k : positive := 1;
  begin
    for i in x'range loop
      r(k) := std_ulogic'image(x(i))(2);
      k := k + 1;
    end loop;
    return r;
  end function;
begin
  process
    type vals_t is array (natural range <>) of std_ulogic_vector(3 downto 0);
    constant vals : vals_t := ("0000", "0001", "0010", "0110", "1111", "XXXX", "UU01", "--01",
                               "000L", "H111", "L000", "000H", "ZZZZ", "WWWW", "0X10", "1110");
    variable x : std_logic_vector(62 downto 0);
    variable y : std_logic_vector(63 downto 0);
    variable w : std_ulogic_vector(7 downto 0);
  begin
    for i in vals'range loop
      s <= vals(i);
      u <= vals(i);
      wait for 1 ns;
      report img(vals(i)) & " " & integer'image(f(vals(i))) & " " & integer'image(f(s)) & " " & integer'image(up(u));
      -- directly on a signal
      case s is
        when "0110" => report "signal 0110";
        when "XXXX" => report "signal XXXX";
        when "1111" | "0000" => report "signal ones or zeros";
        when others => null;
      end case;
    end loop;
    -- a slice, a value through a type conversion
    w := "10110010";
    case w(5 downto 2) is
      when "1100" => report "slice 1100";
      when others => report "slice other";
    end case;
    n <= x"A5";
    wait for 1 ns;
    case n is
      when x"A5" => report "unsigned A5";
      when x"5A" => report "unsigned 5A";
      when others => report "unsigned other";
    end case;
    case std_ulogic_vector(n(3 downto 0)) is
      when "0101" => report "conv 0101";
      when others => report "conv other";
    end case;
    x := (62 => '1', others => '0');
    report "g " & integer'image(g(x));
    x := (0 => '1', others => '0');
    report "g " & integer'image(g(x));
    x := (others => '1');
    report "g " & integer'image(g(x));
    x(5) := 'H';
    report "g " & integer'image(g(x));
    y := (63 => '1', others => '0');
    report "g64 " & integer'image(g64(y));
    y := (others => '1');
    report "g64 " & integer'image(g64(y));
    y(63) := 'X';
    report "g64 " & integer'image(g64(y));
    report "b " & integer'image(b("001")) & " " & integer'image(b("100")) & " " &
           integer'image(b("111")) & " " & integer'image(b("010"));
    report "total " & integer'image(total("00")) & " " & integer'image(total("01")) & " " &
           integer'image(total("10")) & " " & integer'image(total("11"));
    report "up " & integer'image(up("0001")) & " " & integer'image(up("1000")) & " " &
           integer'image(up("HLHL")) & " " & integer'image(up("1010"));
    wait;
  end process;
end architecture;
