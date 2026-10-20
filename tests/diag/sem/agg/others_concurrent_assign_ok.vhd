-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.3.3
-- An `others` aggregate in a concurrent signal assignment takes its bounds from the
-- constrained target signal, also in a conditional and a selected assignment.
library ieee;
use ieee.std_logic_1164.all;
entity others_concurrent_assign_ok is
  port (en : in std_logic; sel : in integer range 0 to 1);
end entity;
architecture a of others_concurrent_assign_ok is
  signal s, t, u : std_logic_vector(3 downto 0);
begin
  s <= (others => 'Z');
  t <= (others => '1') when en = '1' else (others => '0');
  with sel select u <= (others => '0') when 0, (others => '1') when others;
end architecture;
