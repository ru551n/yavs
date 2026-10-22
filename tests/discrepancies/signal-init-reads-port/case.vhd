-- The initial value of a signal reads an `in` port, a vector slice of one, and a signal
-- declared before it.
library ieee;
use ieee.std_logic_1164.all;

entity sub is
  port (a : in std_logic; v : in std_logic_vector(3 downto 0));
end entity;

architecture rtl of sub is
  signal reg  : std_logic := a;
  signal sl   : std_logic_vector(1 downto 0) := v(2 downto 1);
  signal loc  : std_logic := '1';
  signal loc2 : std_logic := not loc;
begin
  process begin
    report "RES " & std_logic'image(reg) & " " & to_string(sl) & " " & std_logic'image(loc2);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity sigport is end entity;

architecture test of sigport is
  signal s  : std_logic := '1';
  signal sv : std_logic_vector(3 downto 0) := "1010";
begin
  u : entity work.sub port map (a => s, v => sv);
end architecture;
