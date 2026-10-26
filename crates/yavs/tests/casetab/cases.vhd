-- Pure functions of one character, enumeration or small integer type that pick their result
-- with a case or an if chain: calls become reads of a result table (#722). The run must be
-- the same as without the tables: results, reports (a value that reports is called), order.
library ieee; use ieee.std_logic_1164.all;
package casetab_cases_pkg is
  type op_t is (op_nop, op_add, op_sub, op_and, op_or, op_xor, op_shl, op_shr, op_not, op_neg);
  type byte_t is range 0 to 255;
  type dig_t is range 0 to 20;
  function hexval(c : character) return integer;
  function hexcheck(c : character) return integer;
  function nibble(c : character) return std_ulogic_vector;
  function nibble_k(c : character) return std_ulogic_vector;
  function opcode(o : op_t) return integer;
  function opwarn(o : op_t) return integer;
  function classify(b : byte_t) return integer;
  function lanes(o : op_t) return bit_vector;
  function digvec(d : dig_t) return std_ulogic_vector;
  function digsum(n : integer) return integer;
end package;
package body casetab_cases_pkg is
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
  -- reports for every character that is not a hex digit
  function hexcheck(c : character) return integer is
  begin
    case c is
      when '0' => return 0; when '1' => return 1; when '2' => return 2; when '3' => return 3;
      when '4' => return 4; when '5' => return 5; when '6' => return 6; when '7' => return 7;
      when '8' => return 8; when '9' => return 9;
      when 'a' | 'A' => return 10; when 'b' | 'B' => return 11; when 'c' | 'C' => return 12;
      when 'd' | 'D' => return 13; when 'e' | 'E' => return 14; when 'f' | 'F' => return 15;
      when others =>
        report "hexcheck: bad character " & integer'image(character'pos(c)) severity warning;
        return -1;
    end case;
  end function;
  -- the shape of a hex-to-nibble function: a variable, an aggregate per arm
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
  -- the result is a constant of each arm
  function nibble_k(c : character) return std_ulogic_vector is
  begin
    case c is
      when '0' => return "0000"; when '1' => return "0001"; when '2' => return "0010";
      when '3' => return "0011"; when '4' => return "0100"; when '5' => return "0101";
      when '6' => return "0110"; when '7' => return "0111"; when '8' => return "1000";
      when '9' => return "1001";
      when 'A' | 'a' => return "1010"; when 'B' | 'b' => return "1011";
      when 'C' | 'c' => return "1100"; when 'D' | 'd' => return "1101";
      when 'E' | 'e' => return "1110"; when 'F' | 'f' => return "1111";
      when others => return "ZZZZ";
    end case;
  end function;
  function opcode(o : op_t) return integer is
  begin
    case o is
      when op_nop => return 0; when op_add => return 10; when op_sub => return 20;
      when op_and => return 30; when op_or => return 40; when op_xor => return 50;
      when op_shl => return 60; when op_shr => return 70; when op_not => return 80;
      when op_neg => return 90;
    end case;
  end function;
  -- reports for op_sub and op_shr
  function opwarn(o : op_t) return integer is
  begin
    case o is
      when op_nop => return 1; when op_add => return 2;
      when op_sub => report "opwarn: sub" severity warning; return 3;
      when op_and => return 4; when op_or => return 5; when op_xor => return 6;
      when op_shl => return 7;
      when op_shr => report "opwarn: shr" severity note; return 8;
      when op_not => return 9; when others => return 10;
    end case;
  end function;
  function classify(b : byte_t) return integer is
  begin
    if b < 10 then return integer(b) * 3;
    elsif b < 32 then return integer(b) + 100;
    elsif b = 32 then return -1;
    elsif b < 48 then return integer(b) - 7;
    elsif b < 58 then return 1000 + integer(b);
    elsif b < 65 then return 2;
    elsif b < 91 then return integer(b) * 2 - 130;
    elsif b < 97 then return 3;
    elsif b < 123 then return integer(b) * 2 - 194;
    elsif b < 200 then return 4;
    else return integer(b) mod 7;
    end if;
  end function;
  -- arrays of different lengths: no table
  function lanes(o : op_t) return bit_vector is
  begin
    case o is
      when op_nop => return "0"; when op_add => return "01"; when op_sub => return "011";
      when op_and => return "0111"; when op_or => return "01111"; when op_xor => return "011111";
      when op_shl => return "0111111"; when op_shr => return "01111111";
      when op_not => return "011111111"; when op_neg => return "0111111111";
    end case;
  end function;
  -- an array result by a value of an integer type, and a mask: the digits 16 to 20 report
  function digvec(d : dig_t) return std_ulogic_vector is
    variable r : std_ulogic_vector(0 to 3);
  begin
    case d is
      when 0 => r := "0000"; when 1 => r := "0001"; when 2 => r := "0010";
      when 3 => r := "0011"; when 4 => r := "0100"; when 5 => r := "0101";
      when 6 => r := "0110"; when 7 => r := "0111"; when 8 => r := "1000";
      when 9 => r := "1001"; when 10 => r := "1010"; when 11 => r := "1011";
      when 12 => r := "1100"; when 13 => r := "1101"; when 14 => r := "1110";
      when 15 => r := "1111";
      when others => report "digvec: bad digit " & integer'image(integer(d)) severity warning;
        r := "UUUU";
    end case;
    return r;
  end function;
  -- the read inside a function, its rest of the block a loop body with a return
  function digsum(n : integer) return integer is
    variable v : std_ulogic_vector(0 to 7);
    variable s : integer := 0;
  begin
    for i in 0 to n loop
      v(4 to 7) := digvec(dig_t(i mod 21));
      if v(7) = '1' then s := s + i; end if;
      for j in 4 to 7 loop
        if v(j) = '1' then s := s + 2 * j; end if;
      end loop;
    end loop;
    return s;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.casetab_cases_pkg.all;
entity casetab_cases is end entity;
architecture t of casetab_cases is
  function to_i(v : std_ulogic_vector) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      case v(i) is
        when '1' => s := s * 3 + 1;
        when '0' => s := s * 3;
        when 'X' => s := s * 3 + 2;
        when others => s := s * 3 + 5;
      end case;
    end loop;
    return s;
  end function;
begin
  process
    variable a, b, c, d, e, f, g, h, m, k : integer := 0;
    variable l : std_ulogic_vector(0 to 15);
    constant hexdigits : string := "0123456789abcdef";
  begin
    for rep in 1 to 3 loop
      for i in 0 to 255 loop
        a := a + hexval(character'val(i)) * (i mod 5 + 1);
        c := c + hexcheck(character'val(i)) * (i mod 3 + 1);
        l(0 to 3) := nibble(character'val(i));
        l(4 to 7) := nibble_k(character'val(i));
        d := d + to_i(l(0 to 7)) + to_i(nibble(character'val(i))) * 2;
        g := g + classify(byte_t(i));
      end loop;
      for o in op_t loop
        b := b + opcode(o) * (op_t'pos(o) + 1);
        e := e + opwarn(o);
        f := f + lanes(o)'length;
      end loop;
    end loop;
    for i in 0 to 20 loop
      l(8 to 11) := digvec(dig_t(i));
      k := to_i(digvec(dig_t(20 - i)));
      m := (m * 5 + to_i(l(8 to 11)) + k) mod 100003;
      l(12 to 15) := digvec(dig_t(i));
    end loop;
    m := m + to_i(l(8 to 15)) + digsum(50);
    -- every result, not only their sums
    for i in 0 to 255 loop
      report integer'image(i) & ":" & to_string(nibble(character'val(i))) & ","
        & to_string(nibble_k(character'val(i))) & "," & integer'image(hexval(character'val(i)))
        & "," & integer'image(classify(byte_t(i)));
    end loop;
    -- one character at a time, as a parser calls them
    for i in 0 to 15 loop
      h := h * 3 + to_i(nibble(hexdigits(i + 1)));
    end loop;
    report "casetab a=" & integer'image(a) & " b=" & integer'image(b) & " c=" & integer'image(c)
      & " d=" & integer'image(d) & " e=" & integer'image(e) & " f=" & integer'image(f)
      & " g=" & integer'image(g) & " h=" & integer'image(h) & " m=" & integer'image(m);
    wait;
  end process;
end architecture;
