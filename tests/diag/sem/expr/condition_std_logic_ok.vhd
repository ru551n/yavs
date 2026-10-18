-- yavs-test: std=2008
-- expect: none
-- LRM 9.2.9
library ieee;
use ieee.std_logic_1164.all;
entity e is end entity;
architecture a of e is
  signal s : std_logic;
begin
  process begin
    if s then null; end if;
    wait;
  end process;
end architecture;
