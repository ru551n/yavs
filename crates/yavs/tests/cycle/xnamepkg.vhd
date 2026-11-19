-- Cycle mode: `xname.vhd`'s design read through an external name in a package procedure, which
-- the scan of outside reads does not follow: every signal of the design is exported.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname_dut is
  port (clk : in std_ulogic; q : out unsigned(7 downto 0));
end entity;
architecture a of cycle_xname_dut is
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal hid : unsigned(7 downto 0) := x"5A";
  signal dbl, fast : unsigned(7 downto 0);
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 3;
      hid <= hid xor cnt;
    end if;
  end process;
  dbl <= cnt + hid;
  fast <= cnt + 1;
  q <= hid;
end architecture;

library ieee;
use ieee.numeric_std.all;
package cycle_xname_peek is
  procedure peek(variable v : out natural);
end package;
package body cycle_xname_peek is
  procedure peek(variable v : out natural) is
  begin
    v := to_integer(<<signal .cycle_xname_pkg.dut.hid : unsigned(7 downto 0)>>);
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_xname_peek.all;
entity cycle_xname_pkg is
end entity;
architecture a of cycle_xname_pkg is
  signal clk : std_ulogic := '0';
  signal q : unsigned(7 downto 0);
begin
  dut : entity work.cycle_xname_dut port map (clk => clk, q => open);
  main : process
    variable sum, v : natural := 0;
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      peek(v);
      sum := sum + v;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & to_string(sum);
    wait;
  end process;
end architecture;
