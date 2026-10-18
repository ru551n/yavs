-- rsim-test: std=2008
-- expect: E1340 12:8 condition must be BOOLEAN, found `std_ulogic`
-- help: no `??` is visible for `std_ulogic`; is `ieee.std_logic_1164` used?
-- LRM 9.2.9
library ieee;
use ieee.std_logic_1164.std_ulogic;
entity e is end entity;
architecture a of e is
  signal s : std_ulogic;
begin
  process begin
    if s then null; end if;
    wait;
  end process;
end architecture;
