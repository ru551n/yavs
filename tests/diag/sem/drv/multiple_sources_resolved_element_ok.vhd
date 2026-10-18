-- yavs-test: std=2008
-- expect: none
-- LRM 4.6, 6.4.2.3: std_logic_vector is (resolved) std_ulogic_vector, so every element is resolved
library ieee;
use ieee.std_logic_1164.all;
entity e is end entity;
architecture a of e is
  subtype sync_t is std_logic_vector(0 to 3);
  signal v : std_logic_vector(3 downto 0);
  signal r : sync_t;
begin
  v(0) <= '1';
  v <= (others => 'L');
  r(1) <= 'Z';
  r <= (others => 'L');
end architecture;
