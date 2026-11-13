-- An adder-tree stage as a function on an unconstrained array of vectors (passed as data and
-- bounds, `Param::flat`), declared in the process, its result sized from the argument's length:
-- a process copy calls a copy of the function for the bounds it passes (#1122), which folds the
-- lengths, the local's descriptor and the positions.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package sub_flat_p is
  type arr_t is array (natural range <>) of signed(7 downto 0);
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.sub_flat_p.all;

entity sub_flat_unit is
  generic (SEED : integer);
  port (clk : in bit; q : out signed(7 downto 0));
end entity;

architecture rtl of sub_flat_unit is
  signal t  : arr_t(0 to 6) := (others => (others => '0'));
  signal l1 : arr_t(0 to 3) := (others => (others => '0'));
  signal l2 : arr_t(0 to 1) := (others => (others => '0'));
begin
  process (clk) is
    function reduce (src : arr_t) return arr_t is
      variable r : arr_t(0 to (src'length + 1) / 2 - 1);
    begin
      for k in r'range loop
        if 2 * k + 1 <= src'high then
          r(k) := src(2 * k) + src(2 * k + 1);
        else
          r(k) := src(2 * k);
        end if;
      end loop;
      return r;
    end function;
  begin
    if clk = '1' then
      for k in t'range loop
        t(k) <= t(k) + to_signed(SEED + 3 * k + 1, 8);
      end loop;
      l1 <= reduce(t);
      l2 <= reduce(l1);
      q <= l2(0) + l2(1);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.sub_flat_p.all;

entity sub_flat_bounds is end entity;

architecture tb of sub_flat_bounds is
  signal clk : bit := '0';
  signal q   : arr_t(0 to 2);
begin
  clk <= not clk after 5 ns;
  g : for i in 0 to 2 generate
    u : entity work.sub_flat_unit generic map (SEED => 10 * i + 3) port map (clk => clk, q => q(i));
  end generate;
  process is
    variable sum : integer := 0;
  begin
    for i in 1 to 300 loop
      wait until clk = '1';
    end loop;
    for i in q'range loop
      sum := sum + to_integer(q(i));
    end loop;
    report "sum=" & integer'image(sum);
    std.env.finish;
  end process;
end architecture;
