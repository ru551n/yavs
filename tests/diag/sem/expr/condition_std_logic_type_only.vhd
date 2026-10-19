-- yavs-test: std=2008
-- expect: E1340 11:8 condition is not BOOLEAN
-- LRM 9.2.9 (silent on where the implicit `??` is looked up; only a visible one counts: majority rule, user 2026-10-19)
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
