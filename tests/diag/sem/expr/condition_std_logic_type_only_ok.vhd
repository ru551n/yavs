-- yavs-test: std=2008
-- expect: none
-- LRM 9.2.9 (silent on where the implicit `??` is looked up; permissive: user 2026-10-19, rule 15)
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
