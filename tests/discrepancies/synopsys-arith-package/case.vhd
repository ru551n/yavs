-- Not in the LRM: use of the non-standard std_logic_arith package.
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
entity top is end entity;
architecture a of top is
  signal v : std_logic_vector(3 downto 0);
begin
  v <= v + 1;
end architecture;
