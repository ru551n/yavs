library ieee;
use ieee.std_logic_1164.all;

package p is
  function inv_res(s : std_ulogic_vector) return std_ulogic;
  subtype inv_vec is (inv_res) std_ulogic_vector;
end package;

package body p is
  function inv_res(s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      r := r xor s(i);
    end loop;
    return not r;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.p.all;

entity leaf is
  port (q : out inv_vec(1 downto 0));
end entity;

architecture a of leaf is
begin
  q <= "10";
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity top is end;

architecture a of top is
  signal v : std_ulogic_vector(1 downto 0);
begin
  u : entity work.leaf port map (q => v);
  process begin
    wait for 1 ns;
    report "RES " & to_string(v);
    wait;
  end process;
end;
