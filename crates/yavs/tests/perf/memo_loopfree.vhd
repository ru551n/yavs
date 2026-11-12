-- Pure functions called with arguments that repeat: `pick` has no loop, recursion, concatenation
-- of bytes or native call but `ieee` gates (compiled callers call it without the memo's
-- probe); `half` has no loop but calls itself (compiled callers probe the memo first).
library ieee; use ieee.std_logic_1164.all;
package memo_loopfree_pkg is
  function ones(v : std_ulogic_vector) return natural;
  function pick(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector;
  function half(v : std_ulogic_vector) return std_ulogic;
end package;
package body memo_loopfree_pkg is
  function ones(v : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then n := n + 1; end if;
    end loop;
    return n;
  end;
  function pick(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
    variable r : std_ulogic_vector(7 downto 0);
  begin
    case op is
      when "000" => r := a and b;
      when "001" => r := a or b;
      when "010" => r := a xor b;
      when "011" => r := not a;
      when "100" => r := a nor b;
      when "101" => r := (a nand b) xor b;
      when "110" => r := a xnor b;
      when others => r := b;
    end case;
    return r;
  end;
  function half(v : std_ulogic_vector) return std_ulogic is
    alias w : std_ulogic_vector(v'length - 1 downto 0) is v;
  begin
    if w'length = 1 then
      return w(0);
    end if;
    return half(w(w'left downto w'length / 2)) xor half(w(w'length / 2 - 1 downto 0));
  end;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.memo_loopfree_pkg.all;
entity memo_loopfree is end;
architecture a of memo_loopfree is
begin
  process
    variable a, b, r : std_ulogic_vector(7 downto 0);
    variable s : natural := 0;
  begin
    for i in 0 to 399 loop
      a := std_ulogic_vector(to_unsigned(i mod 5 * 37, 8));
      b := std_ulogic_vector(to_unsigned(i mod 5 * 11 + 3, 8));
      r := pick(std_ulogic_vector(to_unsigned(i mod 8, 3)), a, b);
      s := s + ones(r) + ones(a & b);
      if half(a) = '1' then s := s + 1; end if;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
