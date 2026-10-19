library ieee;
use ieee.std_logic_1164.all;

entity drive is end entity;

architecture sim of drive is
  signal s_free : std_logic_vector(3 downto 0) := "0000";
  signal s_drv  : std_logic := '0';
  signal s_int  : integer := 0;
  signal s_hold : integer := 0;
begin
  -- s_free and s_int have no driver process; s_drv toggles every 10 ns.
  drv : process begin
    for i in 1 to 8 loop
      wait for 7 ns;
      s_drv <= not s_drv;
    end loop;
    wait;
  end process;

  hold : process (s_int) begin
    s_hold <= s_int + 1;
  end process;
end architecture;
