-- Constants a plain elaboration evaluates only when something needs it, read only by region code:
-- an architecture constant computed by a function and a deferred package constant.
library ieee;
use ieee.std_logic_1164.all;
package cycle_consts_pkg is
  constant deferred_c : std_ulogic_vector(7 downto 0);
  function mk (n : natural) return std_ulogic_vector;
end package;

package body cycle_consts_pkg is
  function mk (n : natural) return std_ulogic_vector is
    variable v : std_ulogic_vector(n - 1 downto 0) := (others => '0');
  begin
    v(0) := '1';
    v(n - 1) := '1';
    return v;
  end function;
  constant deferred_c : std_ulogic_vector(7 downto 0) := mk(8) xor x"A5";
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_consts_pkg.all;
entity cycle_consts is
end entity;
architecture a of cycle_consts is
  constant arch_c : std_ulogic_vector(7 downto 0) := mk(8) or x"30";
  constant width_c : natural := arch_c'length / 2 + deferred_c'length / 4;
  signal a, o : std_ulogic_vector(7 downto 0) := x"00";
  signal h : std_ulogic_vector(width_c - 1 downto 0);
begin
  mix : process (all)
  begin
    o <= a xor arch_c xor deferred_c;
    h <= a(width_c - 1 downto 0);
  end process;

  main : process
  begin
    for c in 1 to 4 loop
      a <= std_ulogic_vector'(x"3C") rol c;
      wait for 5 ns;
      report "o=" & to_hstring(o) & " h=" & to_hstring(h);
    end loop;
    wait;
  end process;
end architecture;
