-- difftest: top=record_in_port_bounds std=2008
-- Bounds of std_logic_vector elements of an `in` record port during elaboration (#304):
-- an architecture constant and a process variable sized by `p.wdata'length`, the shape of
-- a bus functional model's slave interface.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package record_in_port_bounds_pkg is
  type wr_in_t is record
    awaddr : std_logic_vector;
    wdata  : std_logic_vector;
    wvalid : std_logic;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_in_port_bounds_pkg.all;
entity record_in_port_bounds_slave is
  port (clk : in std_logic; wr_in : in wr_in_t);
end entity;

architecture a of record_in_port_bounds_slave is
  constant C_WDATA_WIDTH : natural := wr_in.wdata'length;
  signal wdata : std_logic_vector(C_WDATA_WIDTH - 1 downto 0);
begin
  process
    variable v_addr : unsigned(wr_in.awaddr'length - 1 downto 0);
  begin
    wait until rising_edge(clk) and wr_in.wvalid = '1';
    v_addr := unsigned(wr_in.awaddr);
    wdata <= wr_in.wdata;
    wait for 0 ns;
    report "w " & integer'image(C_WDATA_WIDTH) & " a " & integer'image(v_addr'length)
      & " " & integer'image(to_integer(v_addr)) & " " & to_hstring(wdata);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.record_in_port_bounds_pkg.all;
entity record_in_port_bounds is end entity;

architecture t of record_in_port_bounds is
  signal clk : std_logic := '0';
  signal w : wr_in_t(awaddr(11 downto 0), wdata(15 downto 0));
begin
  s : entity work.record_in_port_bounds_slave port map (clk => clk, wr_in => w);
  process begin
    w <= (awaddr => x"0a5", wdata => x"beef", wvalid => '1');
    wait for 5 ns;
    clk <= '1';
    wait;
  end process;
end architecture;
