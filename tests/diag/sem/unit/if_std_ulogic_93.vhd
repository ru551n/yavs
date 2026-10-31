-- yavs-test: std=1993
-- expect: E1340 8:8 condition is not BOOLEAN
-- 1993 has no `??`: a plain condition stays an error; only a PSL boolean is taken (#911)
library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture a of e is signal x : std_logic; begin
  process begin
    if x then null; end if;
    wait;
  end process;
end;
