-- Cycle mode with a region that does not take every process of the design under test: `mix`
-- and `reg` join, `pick` (an `if`, not supported yet) and the concurrent `q <= r` stay in the
-- event kernel. Values cross the boundary both ways: `a` from the region to `pick`, `b` back from
-- `pick` into the region, `din` from the testbench into a combinational member (it wakes the
-- region), `r` out through `q`. `n` is read by the clocked process that writes it.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_partial_dut is
  port (
    clk : in std_ulogic;
    din : in std_ulogic_vector(7 downto 0);
    q : out std_ulogic_vector(7 downto 0)
  );
end entity;

architecture rtl of cycle_partial_dut is
  signal r : std_ulogic_vector(7 downto 0) := x"5A";
  signal a : std_ulogic_vector(7 downto 0) := x"00";
  signal b : std_ulogic_vector(7 downto 0) := x"00";
  signal n : unsigned(7 downto 0) := x"00";
begin
  mix : process (r, din)
  begin
    a <= r xor din;
  end process;

  pick : process (a)
  begin
    if a(0) = '1' then
      b <= a(3 downto 0) & a(7 downto 4);
    else
      b <= not a;
    end if;
  end process;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= a xor b xor std_ulogic_vector(n);
      n <= n + 3;
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
begin
  dut : entity work.cycle_partial_dut port map (clk => clk, din => din, q => q);

  main : process
    variable acc : natural := 0;
  begin
    for i in 1 to CYCLES loop
      din <= std_ulogic_vector(to_unsigned((i * 37) mod 256, 8));
      wait for 2 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      acc := (acc * 31 + to_integer(unsigned(q))) mod 1000003;
      wait for 3 ns;
    end loop;
    report "acc=" & integer'image(acc) & " q=" & to_hstring(q);
    wait;
  end process;
end architecture;
