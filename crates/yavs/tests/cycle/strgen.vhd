-- A configuration test on a string generic in a clocked process: a pure function with `while`
-- loops over strings, which elaboration evaluates per process and region code folds.
library ieee;
use ieee.std_logic_1164.all;
package cycle_strgen_pkg is
  function same_word(a, b : string) return boolean;
end package;
package body cycle_strgen_pkg is
  function upper(a : string) return string is
    variable r : string(1 to a'length) := a;
  begin
    for i in r'range loop
      if r(i) >= 'a' and r(i) <= 'z' then
        r(i) := character'val(character'pos(r(i)) - 32);
      end if;
    end loop;
    return r;
  end function;
  function trim(a : string) return string is
    variable s : natural := a'left;
    variable e : natural := a'right;
  begin
    while s < a'right and a(s) = ' ' loop
      s := s + 1;
    end loop;
    while e > a'left and a(e) = ' ' loop
      e := e - 1;
    end loop;
    return a(s to e);
  end function;
  function same_word(a, b : string) return boolean is
  begin
    return upper(trim(a)) = upper(trim(b));
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_strgen_pkg.all;
entity cycle_strgen_u is
  generic (MODE : string);
  port (clk, a : in std_ulogic; q : out std_ulogic);
end entity;
architecture a of cycle_strgen_u is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if same_word(MODE, " rbw  ") then
        q <= a;
      else
        q <= not a;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_strgen is
end entity;
architecture t of cycle_strgen is
  signal clk, a, q1, q2 : std_ulogic := '0';
begin
  u1 : entity work.cycle_strgen_u generic map (MODE => "RBW") port map (clk => clk, a => a, q => q1);
  u2 : entity work.cycle_strgen_u generic map (MODE => "WBR") port map (clk => clk, a => a, q => q2);
  main : process
  begin
    for k in 1 to 6 loop
      a <= '1' when k mod 2 = 0 else '0';
      wait for 1 ns;
      clk <= '1';
      wait for 4 ns;
      clk <= '0';
      wait for 5 ns;
      report "k=" & integer'image(k) & " q1=" & std_ulogic'image(q1) & " q2=" & std_ulogic'image(q2);
    end loop;
    wait;
  end process;
end architecture;
