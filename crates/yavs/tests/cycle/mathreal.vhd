-- Sizes from `math_real` (`ceil(log2(real(n)))` in a function of the design, with an assert
-- that holds), folded where the arguments are known, against the normal mode (`W` replaced by a
-- width): a shift count in a register, a loop bound and a loop parameter's size in a
-- combinational process (both join); a function returning a vector (`to_size`, whose `log2`
-- asserts) stays in the event kernel.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;
package cycle_mathreal_pkg is
  function ceil_log2(value : positive) return natural;
  function log2(value : positive) return natural;
  function to_size(width : positive) return std_ulogic_vector;
end package;
package body cycle_mathreal_pkg is
  function ceil_log2(value : positive) return natural is
  begin
    return natural(ceil(log2(real(value))));
  end function;
  function log2(value : positive) return natural is
  begin
    assert 2 ** natural(floor(log2(real(value)))) = value report "not a power of two" severity failure;
    return natural(floor(log2(real(value))));
  end function;
  function to_size(width : positive) return std_ulogic_vector is
  begin
    return std_ulogic_vector(to_unsigned(log2(width / 8), 3));
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_mathreal_pkg.all;
entity cycle_mathreal_u is
  generic (DW : positive; N : positive);
  port (clk : in std_ulogic; x : in unsigned(W + 15 downto 0); size : out std_ulogic_vector(2 downto 0);
        q : out unsigned(W + 15 downto 0); c : out natural);
end entity;
architecture a of cycle_mathreal_u is
begin
  size <= to_size(DW);
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= shift_right(x, ceil_log2(N));
    end if;
  end process;
  -- The argument is a loop parameter: folded in each unrolled iteration.
  ones : process (all)
    variable n1 : natural;
  begin
    n1 := 0;
    for k in 0 to ceil_log2(N) loop
      if x(k) = '1' then
        n1 := n1 + ceil_log2(k + 3);
      end if;
    end loop;
    c <= n1;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_mathreal is
end entity;
architecture t of cycle_mathreal is
  signal clk : std_ulogic := '0';
  signal x, q : unsigned(W + 15 downto 0) := (others => '0');
  signal size : std_ulogic_vector(2 downto 0);
  signal c : natural;
begin
  u : entity work.cycle_mathreal_u generic map (DW => 64, N => 13)
    port map (clk => clk, x => x, size => size, q => q, c => c);
  main : process
    variable s : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 10 loop
      s := s(14 downto 0) & (s(15) xor s(13) xor s(12) xor s(10));
      x <= x(W + 14 downto 0) & s(0);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " size=" & to_string(size) & " q=" & to_hstring(q)
        & " c=" & integer'image(c);
    end loop;
    wait;
  end process;
end architecture;
