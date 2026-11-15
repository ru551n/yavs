-- Bounds `0 to x'length - 1` and `x'length - 1 downto 0` of a parameter whose bounds are read in
-- registers: their length is `x'length`, so the copy into them checks no length.
library ieee;
use ieee.std_logic_1164.all;
entity len_reg_bounds is end;
architecture t of len_reg_bounds is
  procedure put(variable u : inout std_ulogic_vector(0 to 31); x : std_ulogic_vector;
                variable n : inout natural) is
    variable t : std_ulogic_vector(x'length - 1 downto 0);
  begin
    t := x;
    u(0 to x'length - 1) := t;
    for k in t'range loop
      if t(k) = '1' then
        n := n + 1;
      end if;
    end loop;
  end procedure;
begin
  process
    variable u : std_ulogic_vector(0 to 31) := (others => '0');
    variable w : std_ulogic_vector(11 downto 0) := x"5a3";
    variable acc : natural := 0;
  begin
    for n in 0 to 39 loop
      w := w(10 downto 0) & w(11);
      put(u, w, acc);
      put(u, w(7 downto 0), acc);
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
