-- Cycle mode: more places of external names into `xname.vhd`'s design. `cycle_xname_wait`
-- waits on one directly and reads one in a concurrent assertion: only those are exported.
-- `cycle_xname_fn` reads one in a function of the architecture and `cycle_xname_lpkg` in a
-- package declared in the architecture: the scan does not place those, every leaf is exported.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname_wait is
end entity;
architecture a of cycle_xname_wait is
  signal clk : std_ulogic := '0';
  signal changes : natural := 0;
begin
  dut : entity work.cycle_xname_dut port map (clk => clk, q => open);
  main : process
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "fast=" & to_string(changes);
    wait;
  end process;
  watch : process
  begin
    wait on <<signal .cycle_xname_wait.dut.fast : unsigned(7 downto 0)>>;
    changes <= changes + 1;
  end process;
  assert <<signal .cycle_xname_wait.dut.dbl : unsigned(7 downto 0)>> /= x"FF"
    report "dbl=FF" severity note;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname_fn is
end entity;
architecture a of cycle_xname_fn is
  signal clk : std_ulogic := '0';
  impure function peek return natural is
  begin
    return to_integer(<<signal .cycle_xname_fn.dut.hid : unsigned(7 downto 0)>>);
  end function;
begin
  dut : entity work.cycle_xname_dut port map (clk => clk, q => open);
  main : process
    variable sum : natural := 0;
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      sum := sum + peek;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & to_string(sum);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname_lpkg is
end entity;
architecture a of cycle_xname_lpkg is
  signal clk : std_ulogic := '0';
  package peeks is
    impure function peek return natural;
  end package;
  package body peeks is
    impure function peek return natural is
    begin
      return to_integer(<<signal .cycle_xname_lpkg.dut.hid : unsigned(7 downto 0)>>);
    end function;
  end package body;
  use peeks.all;
begin
  dut : entity work.cycle_xname_dut port map (clk => clk, q => open);
  main : process
    variable sum : natural := 0;
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      sum := sum + peek;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & to_string(sum);
    wait;
  end process;
end architecture;
