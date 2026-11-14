-- difftest: top=element_results std=2008
-- Results of numeric_std and std_logic_1164 operations copied into an element of an array of
-- vectors: rows of 5, 12 (rows that straddle a word of the planes), 16 and 64 elements, ascending,
-- descending and negative index ranges, an element that is an operand of its own result
-- (`r(k) := r(k) + r(k - 1)`, `r(k) := r(k) + r(k)`), metavalues in the operands, a truncated
-- `to_unsigned` (its warning), a destination index through `**`, in functions, in the process,
-- into a signal and through a signal parameter of a procedure.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package element_results_pkg is
  type a5 is array (natural range <>) of signed(4 downto 0);
  type a12 is array (natural range <>) of unsigned(11 downto 0);
  type a16 is array (integer range <>) of signed(15 downto 0);
  type a64 is array (natural range <>) of std_logic_vector(63 downto 0);
  function self5(v : a5) return a5;
  function acc12(v : a12) return a12;
  function down16(v : a16) return a16;
  function gates64(v : a64) return a64;
  function trunc12(v : a12) return a12;
  function pow12(v : a12) return a12;
end package;

package body element_results_pkg is
  function self5(v : a5) return a5 is
    variable r : a5(v'range) := v;
  begin
    for k in r'range loop
      r(k) := r(k) + r(k);
      r(k) := abs r(k);
    end loop;
    return r;
  end function;

  function acc12(v : a12) return a12 is
    variable r : a12(v'range) := v;
  begin
    for k in r'low + 1 to r'high loop
      r(k) := r(k) + r(k - 1);
    end loop;
    return r;
  end function;

  function down16(v : a16) return a16 is
    variable r : a16(v'high - 1 downto v'low);
  begin
    for k in r'range loop
      r(k) := v(k + 1) - v(k);
    end loop;
    return r;
  end function;

  function gates64(v : a64) return a64 is
    variable r : a64(0 to v'length - 2);
  begin
    for k in r'range loop
      r(k) := v(v'low + k) xor not v(v'low + k + 1);
    end loop;
    return r;
  end function;

  function trunc12(v : a12) return a12 is
    variable r : a12(v'range);
  begin
    for k in r'range loop
      r(k) := to_unsigned(to_integer(v(k)(10 downto 0)) * 3, 12);
    end loop;
    return r;
  end function;

  -- The destination index through `**` (a step of the interpreter on a native frame).
  function pow12(v : a12) return a12 is
    variable r : a12(0 to 7) := (others => (others => '0'));
  begin
    for k in 0 to 3 loop
      r(2 ** k - 1) := v(k) + v(k + 1);
    end loop;
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.element_results_pkg.all;

entity element_results is
end entity;

architecture sim of element_results is
  signal sv : a12(0 to 6);
  signal done : boolean := false;

  procedure pairs(signal src : in a12; signal dst : out a12) is
  begin
    for k in dst'range loop
      dst(k) <= src(k) + src(k + 1);
    end loop;
  end procedure;

  signal pv : a12(0 to 5);
begin
  process is
    variable v5 : a5(0 to 6);
    variable v12 : a12(0 to 6);
    variable v16 : a16(3 downto -4);
    variable v64 : a64(1 to 3);
    variable p12 : a12(2 to 8);
    variable s : integer := 0;
  begin
    for i in 0 to 999 loop
      for k in v5'range loop
        v5(k) := to_signed((i * 3 + k) mod 32 - 16, 5);
      end loop;
      for k in v12'range loop
        v12(k) := to_unsigned((i * 37 + k * 11) mod 4096, 12);
      end loop;
      for k in v16'range loop
        v16(k) := to_signed((i * 101 - k * 7) mod 30000 - 15000, 16);
      end loop;
      for k in v64'range loop
        v64(k) := std_logic_vector(to_unsigned(i * 7919 + k, 32)) & std_logic_vector(to_unsigned(i + k * 3, 32));
      end loop;
      if i mod 250 = 7 then
        v5(2)(1) := 'X';
        v12(3)(0) := 'U';
        v16(0)(9) := '-';
        v64(2)(40) := 'W';
      end if;
      v5 := self5(v5);
      v12 := acc12(v12);
      v16(2 downto -4) := down16(v16);
      v64(1 to 2) := gates64(v64);
      if i mod 100 = 3 then
        p12 := trunc12(v12);
        report "t " & to_hstring(p12(2)) & " " & to_hstring(p12(8));
        p12(2 to 8) := pow12(v12)(1 to 7);
        report "w " & to_hstring(p12(2)) & " " & to_hstring(p12(4)) & " " & to_hstring(p12(8));
      end if;
      -- In the process: the same operations on its own variables.
      p12 := v12;
      for k in p12'low + 1 to p12'high loop
        p12(k) := p12(k) + p12(k - 1);
      end loop;
      for k in v16'low to v16'high - 1 loop
        v16(k) := v16(k) - v16(k + 1);
      end loop;
      sv <= v12;
      wait for 1 ns;
      pairs(sv, pv);
      for k in pv'range loop
        sv(k) <= sv(k) + sv(k + 1);
      end loop;
      wait for 1 ns;
      if i mod 50 = 0 or i mod 250 = 7 then
        report integer'image(i) & " " & to_hstring(std_logic_vector(v5(2))) & " " & to_hstring(v12(6)) & " " &
          to_hstring(p12(8)) & " " & to_hstring(std_logic_vector(v16(-4))) & " " & to_hstring(v64(2)) & " " &
          to_hstring(sv(0)) & " " & to_hstring(pv(5));
      end if;
      if not is_x(std_logic_vector(v16(-4))) and not is_x(std_logic_vector(p12(8))) then
        s := (s + to_integer(v16(-4)) + to_integer(p12(8)) + to_integer(sv(0))) mod 1000003;
      end if;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end architecture;
