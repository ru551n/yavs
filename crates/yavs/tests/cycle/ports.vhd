-- Cycle mode: ports collapsed into parts of their actuals (two instances, each on half of a
-- vector) and a package constant, against the normal mode.
library ieee;
use ieee.std_logic_1164.all;
package cycle_ports_pkg is
  constant ROT : natural := 1;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_ports_pkg.all;
entity cycle_ports_sub is
  port (
    clk : in std_ulogic;
    a : in std_ulogic_vector(3 downto 0);
    y : out std_ulogic_vector(3 downto 0)
  );
end entity;
architecture rtl of cycle_ports_sub is
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      y <= a(3 - ROT downto 0) & a(3 downto 4 - ROT);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_ports is
end entity;
architecture tb of cycle_ports is
  signal clk : std_ulogic := '0';
  signal din : std_ulogic_vector(7 downto 0) := x"A5";
  signal dout : std_ulogic_vector(7 downto 0) := x"00";
begin
  lo : entity work.cycle_ports_sub port map (clk => clk, a => din(3 downto 0), y => dout(3 downto 0));
  hi : entity work.cycle_ports_sub port map (clk => clk, a => din(7 downto 4), y => dout(7 downto 4));
  main : process
  begin
    for i in 1 to 6 loop
      din <= din(6 downto 0) & (din(7) xor din(2));
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "i=" & integer'image(i) & " dout=" & to_hstring(dout);
    end loop;
    wait;
  end process;
end architecture;
