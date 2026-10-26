-- Two processes call a pure function on a vector whose argument never repeats: both give its memo
-- up after the first calls, and then it is off for all (#762), though fewer than three processes
-- call it.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package memo_giveup_pkg is
  function rev(v : std_logic_vector) return std_logic_vector;
end package;
package body memo_giveup_pkg is
  function rev(v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'reverse_range);
  begin
    for i in v'range loop
      r(i) := v(i);
    end loop;
    return r;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.memo_giveup_pkg.all;
entity memo_giveup is end;
architecture a of memo_giveup is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal cnt : unsigned(11 downto 0) := (others => '0');
  signal a, b : std_logic_vector(0 to 11);
begin
  clk <= not clk after 5 ns when not done else clk;
  cnt <= cnt + 1 when rising_edge(clk);
  a <= rev(std_logic_vector(cnt));
  b <= rev(std_logic_vector(cnt + 7));
  process
    variable s : natural := 0;
  begin
    for c in 1 to 150 loop
      wait until rising_edge(clk);
      wait for 1 ns;
      s := (s + to_integer(unsigned(a)) + to_integer(unsigned(b))) mod 65536;
    end loop;
    done <= true;
    report "acc=" & integer'image(s);
    wait;
  end process;
end;
