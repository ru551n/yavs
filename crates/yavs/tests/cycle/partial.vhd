-- Cycle mode with a region that does not take every process of the design under test: `mix`,
-- `late`, `early`, `reg` and the concurrent `q <= r` join (`late` reads `early`, so it runs
-- after it); `pick` (a call of a function with a `while` loop, not supported yet) stays in the
-- event kernel. Values cross the boundary both ways: `a` from the region to `pick`, `b` back from `pick` into the region, `din`
-- from the testbench into a combinational member (it wakes the region), `r` out through `q`; the
-- scalars `en` in and `t` out. `n` is read by the clocked process that writes it. The assert of
-- `reg` is not evaluated in cycle mode and does not keep it out.
library ieee;
use ieee.std_logic_1164.all;
package cycle_partial_pkg is
  function swap(a : std_ulogic_vector(7 downto 0)) return std_ulogic_vector;
end package;

package body cycle_partial_pkg is
  function swap(a : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
    variable r : std_ulogic_vector(7 downto 0);
    variable i : natural := 0;
  begin
    while i < 4 loop
      r(i) := a(i + 4);
      r(i + 4) := a(i);
      i := i + 1;
    end loop;
    return r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_partial_pkg.all;

entity cycle_partial_dut is
  port (
    clk : in std_ulogic;
    din : in std_ulogic_vector(7 downto 0);
    en : in std_ulogic;
    q : out std_ulogic_vector(7 downto 0);
    t : out std_ulogic
  );
end entity;

architecture rtl of cycle_partial_dut is
  signal r : std_ulogic_vector(7 downto 0) := x"5A";
  signal a : std_ulogic_vector(7 downto 0) := x"00";
  signal b : std_ulogic_vector(7 downto 0) := x"00";
  signal n : unsigned(7 downto 0) := x"00";
  signal e1, e2 : std_ulogic_vector(7 downto 0) := x"00";
begin
  -- Runs after `early`, which it reads, although it comes first.
  late : process (e1)
  begin
    e2 <= not e1;
  end process;

  early : process (r)
  begin
    e1 <= r(3 downto 0) & r(7 downto 4);
  end process;

  mix : process (r, din)
  begin
    a <= r xor din;
  end process;

  pick : process (a)
  begin
    if a(0) = '1' then
      b <= swap(a);
    else
      b <= not a;
    end if;
  end process;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= a xor b xor std_ulogic_vector(n) xor e2;
      n <= n + 3;
      t <= a(0) xor en;
      -- Not evaluated in cycle mode (it never fails here).
      assert a = (a xor b xor b) report "xor is not its own inverse" severity failure;
    end if;
  end process;

  q <= r;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_partial is
  generic (CYCLES : natural := 300);
end entity;

architecture tb of cycle_partial is
  signal clk : std_ulogic := '0';
  signal din : std_ulogic_vector(7 downto 0) := x"00";
  signal q : std_ulogic_vector(7 downto 0) := x"00";
  signal en, t : std_ulogic := '0';
  signal ts : natural := 0;
begin
  dut : entity work.cycle_partial_dut port map (clk => clk, din => din, en => en, q => q, t => t);

  main : process
    variable acc : natural := 0;
  begin
    for i in 1 to CYCLES loop
      din <= std_ulogic_vector(to_unsigned((i * 37) mod 256, 8));
      en <= '1' when i mod 3 = 0 else '0';
      wait for 2 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      acc := (acc * 31 + to_integer(unsigned(q))) mod 1000003;
      if t = '1' then
        acc := (acc + 7) mod 1000003;
      end if;
      wait for 3 ns;
    end loop;
    report "acc=" & integer'image(acc) & " q=" & to_hstring(q);
    wait;
  end process;
end architecture;
