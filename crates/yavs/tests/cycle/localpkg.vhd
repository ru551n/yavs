-- Cycle mode: a package declared in the testbench architecture reads a signal of the region
-- (`acc`) in an impure function the testbench calls: that signal is read outside and exported.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_localpkg_dut is
  port (clk : in std_ulogic);
end entity;
architecture a of cycle_localpkg_dut is
  signal cnt : unsigned(7 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 3;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_localpkg is
end entity;
architecture a of cycle_localpkg is
  signal clk : std_ulogic := '0';
  signal acc : unsigned(7 downto 0) := (others => '0');
  package peek is
    impure function get return natural;
  end package;
  package body peek is
    impure function get return natural is
    begin
      return to_integer(acc);
    end function;
  end package body;
  use peek.all;
begin
  dut : entity work.cycle_localpkg_dut port map (clk => clk);
  reg : process (clk)
  begin
    if rising_edge(clk) then
      acc <= acc + 5;
    end if;
  end process;
  main : process
    variable sum : natural := 0;
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      sum := sum + get;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & to_string(sum);
    wait;
  end process;
end architecture;
