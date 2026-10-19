-- LRM 9.2.9: where the implicit ?? of a condition comes from, with only the type made visible.
library ieee;
use ieee.std_logic_1164.std_ulogic;
entity top is end entity;
architecture a of top is
  signal s : std_ulogic;
begin
  process begin
    if s then null; end if;
    wait;
  end process;
end architecture;
