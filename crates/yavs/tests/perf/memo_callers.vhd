-- Eight processes call a pure function with a loop on a vector whose argument never repeats:
-- with more callers than votes, their first give-ups turn it off for all (no later trials).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package memo_callers_pkg is
  function rev(v : std_logic_vector) return std_logic_vector;
end package;
package body memo_callers_pkg is
  function rev(v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'reverse_range);
  begin
    for i in v'range loop
      r(i) := v(i);
    end loop;
    return r;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.memo_callers_pkg.all;
entity memo_callers is generic (n : natural := 8; cycles : natural := 1200); end;
architecture a of memo_callers is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal cnt : unsigned(15 downto 0) := (others => '0');
  type arr is array (0 to n - 1) of std_logic_vector(0 to 15);
  signal a : arr;
begin
  clk <= not clk after 5 ns when not done else clk;
  cnt <= cnt + 1 when rising_edge(clk);
  g : for i in 0 to n - 1 generate
    a(i) <= rev(std_logic_vector(cnt + i * 977));
  end generate;
  process
    variable s : natural := 0;
  begin
    for c in 1 to cycles loop
      wait until rising_edge(clk);
      wait for 1 ns;
      for i in 0 to n - 1 loop
        s := (s + to_integer(unsigned(a(i)))) mod 65536;
      end loop;
    end loop;
    done <= true;
    report "acc=" & integer'image(s);
    wait;
  end process;
end;
