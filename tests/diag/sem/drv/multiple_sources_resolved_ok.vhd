-- yavs-test: std=2008
-- expect: none
-- LRM 4.6
library ieee;
use ieee.std_logic_1164.all;
entity e is end entity;
architecture a of e is
  signal s : std_logic;
begin
  s <= '1';
  s <= 'Z';
end architecture;
