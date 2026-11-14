-- difftest: top=element_operands std=2008
-- Elements of arrays of vectors as operands of numeric_std and std_logic_1164 operations in
-- functions: rows of 1, 5, 12, 16, 63 and 64 elements (rows that straddle a word of the planes),
-- ascending and descending index ranges, metavalues, `-`, `L` and `H` in the operands, an element
-- used twice, an element of the result array as an operand of the next one, and the array
-- updated from its own elements.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package element_operands_pkg is
  type a1 is array (natural range <>) of unsigned(0 downto 0);
  type a5 is array (natural range <>) of signed(4 downto 0);
  type a12 is array (natural range <>) of unsigned(11 downto 0);
  type a16 is array (integer range <>) of signed(15 downto 0);
  type a63 is array (natural range <>) of unsigned(62 downto 0);
  type a64 is array (natural range <>) of std_logic_vector(63 downto 0);
  function sums1(v : a1) return integer;
  function sums5(v : a5) return a5;
  function sums12(v : a12) return a12;
  function sums16(v : a16) return a16;
  function sums63(v : a63) return a63;
  function gates64(v : a64) return a64;
  function rel12(v : a12) return integer;
  function walk12(v : a12) return a12;
  function twice16(v : a16) return a16;
end package;

package body element_operands_pkg is
  function sums1(v : a1) return integer is
    variable n : integer := 0;
  begin
    for k in v'low to v'high - 1 loop
      n := n + to_integer(v(k) + v(k + 1));
    end loop;
    return n;
  end function;

  function sums5(v : a5) return a5 is
    variable r : a5(v'range);
  begin
    for k in v'range loop
      if k < v'high then
        r(k) := v(k) - v(k + 1);
      else
        r(k) := abs v(k);
      end if;
    end loop;
    return r;
  end function;

  function sums12(v : a12) return a12 is
    variable r : a12(0 to v'length - 2);
  begin
    for k in r'range loop
      r(k) := v(v'low + k) + v(v'low + k + 1);
    end loop;
    return r;
  end function;

  function sums16(v : a16) return a16 is
    variable r : a16(v'range);
  begin
    for k in v'range loop
      if k = v'left then
        r(k) := v(k);
      elsif v'ascending then
        r(k) := v(k) + v(k - 1);
      else
        r(k) := v(k) - v(k + 1);
      end if;
    end loop;
    return r;
  end function;

  function sums63(v : a63) return a63 is
    variable r : a63(v'range);
  begin
    for k in v'range loop
      r(k) := v(k) + v(v'high - k + v'low);
    end loop;
    return r;
  end function;

  function gates64(v : a64) return a64 is
    variable r : a64(v'range);
  begin
    for k in v'low to v'high - 1 loop
      r(k) := v(k) xor v(k + 1);
    end loop;
    r(v'high) := v(v'high) and v(v'low);
    return r;
  end function;

  function rel12(v : a12) return integer is
    variable n : integer := 0;
  begin
    for k in v'low to v'high - 1 loop
      if v(k) < v(k + 1) then
        n := n + 1;
      end if;
      if v(k) = v(k + 1) then
        n := n + 100;
      end if;
      if std_match(v(k), v(k + 1)) then
        n := n + 10000;
      end if;
    end loop;
    return n;
  end function;

  -- Each element from the one before it, read back from the array being written.
  function walk12(v : a12) return a12 is
    variable r : a12(v'range) := v;
  begin
    for k in r'low + 1 to r'high loop
      r(k) := r(k) + r(k - 1);
    end loop;
    return r;
  end function;

  function twice16(v : a16) return a16 is
    variable r : a16(v'range) := v;
  begin
    for k in r'range loop
      r(k) := r(k) + r(k);
    end loop;
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.element_operands_pkg.all;

entity element_operands is
end entity;

architecture sim of element_operands is
  function hex(v : std_logic_vector) return string is
  begin
    return to_hstring(v);
  end function;
begin
  process is
    variable v1 : a1(0 to 9);
    variable v5 : a5(3 to 9);
    variable v12 : a12(0 to 13);
    variable u16 : a16(-3 to 5);
    variable d16 : a16(7 downto 0);
    variable v63 : a63(1 to 4);
    variable v64 : a64(0 to 4);
    variable r5 : a5(3 to 9);
    variable r12 : a12(0 to 12);
    variable w12 : a12(0 to 13);
    variable r16 : a16(-3 to 5);
    variable s16 : a16(7 downto 0);
    variable r63 : a63(1 to 4);
    variable r64 : a64(0 to 4);
    variable sum : unsigned(31 downto 0);
  begin
    for round in 0 to 2 loop
      for k in v1'range loop
        v1(k) := to_unsigned((k + round) mod 2, 1);
      end loop;
      for k in v5'range loop
        v5(k) := to_signed(k * 5 - 17 + round, 5);
      end loop;
      for k in v12'range loop
        v12(k) := to_unsigned(k * 311 + round * 1000, 12);
      end loop;
      for k in u16'range loop
        u16(k) := to_signed(k * 4099 - 777 * round, 16);
      end loop;
      for k in d16'range loop
        d16(k) := to_signed(k * 12345 - 4000 + round, 16);
      end loop;
      for k in v63'range loop
        v63(k) := shift_left(to_unsigned(k * 977 + round, 63), 40 + k);
      end loop;
      for k in v64'range loop
        v64(k) := std_logic_vector(shift_left(to_unsigned(k * 5 + round + 1, 64), k * 13));
      end loop;
      -- Metavalues in the last round: the operations' own rules for them.
      if round = 2 then
        v12(3)(5) := 'X';
        v12(6) := (others => '-');
        v12(7)(0) := 'L';
        v12(8)(11) := 'H';
        u16(0)(15) := 'U';
        d16(4)(2) := 'Z';
        v64(2)(63) := 'W';
        v64(3)(0) := 'L';
      end if;
      report "round " & integer'image(round) & " sums1 " & integer'image(sums1(v1));
      r5 := sums5(v5);
      for k in r5'range loop
        report "r5 " & integer'image(k) & " " & hex(std_logic_vector(r5(k)));
      end loop;
      r12 := sums12(v12);
      for k in r12'range loop
        report "r12 " & integer'image(k) & " " & hex(std_logic_vector(r12(k)));
      end loop;
      report "rel12 " & integer'image(rel12(v12));
      -- The same operations in the process.
      for k in v12'low to v12'high - 1 loop
        report "p12 " & integer'image(k) & " " & hex(std_logic_vector(v12(k) + v12(k + 1)))
          & " " & boolean'image(v12(k) < v12(k + 1));
      end loop;
      for k in d16'range loop
        report "p16 " & integer'image(k) & " " & hex(std_logic_vector(d16(k) - d16(7 - k)));
      end loop;
      w12 := walk12(v12);
      for k in w12'range loop
        report "w12 " & integer'image(k) & " " & hex(std_logic_vector(w12(k)));
      end loop;
      r16 := sums16(u16);
      for k in r16'range loop
        report "r16 " & integer'image(k) & " " & hex(std_logic_vector(r16(k)));
      end loop;
      s16 := sums16(d16);
      for k in s16'range loop
        report "s16 " & integer'image(k) & " " & hex(std_logic_vector(s16(k)));
      end loop;
      s16 := twice16(d16);
      for k in s16'range loop
        report "t16 " & integer'image(k) & " " & hex(std_logic_vector(s16(k)));
      end loop;
      r63 := sums63(v63);
      for k in r63'range loop
        report "r63 " & integer'image(k) & " " & hex(std_logic_vector(r63(k)));
      end loop;
      r64 := gates64(v64);
      for k in r64'range loop
        report "r64 " & integer'image(k) & " " & hex(r64(k));
      end loop;
    end loop;
    -- Many calls, so tiers that compile hot code run the compiled function too.
    sum := (others => '0');
    for i in 1 to 3000 loop
      v12(i mod 14) := to_unsigned(i, 12);
      r12 := sums12(v12);
      sum := sum + r12(i mod 13);
      u16(i mod 9 - 3) := to_signed(i * 7, 16);
      r16 := sums16(u16);
      sum := sum + unsigned(std_logic_vector(resize(r16(i mod 9 - 3), 32)));
    end loop;
    report "sum " & hex(std_logic_vector(sum));
    wait;
  end process;
end architecture;
