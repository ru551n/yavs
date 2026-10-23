library ieee; use ieee.std_logic_1164.all;
use work.unit_pkg.all; use work.unit_mid.all;
entity unit_tb is end;
architecture a of unit_tb is
begin
  process begin
    report "K=" & integer'image(K) & " M=" & integer'image(M) & " f=" & integer'image(f("0101"));
    wait;
  end process;
end;
