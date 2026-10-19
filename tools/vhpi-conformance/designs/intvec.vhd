-- Targets of an integer put into vectors: undriven signals of several shapes.
library ieee;
use ieee.std_logic_1164.all;

entity intvec is end entity;

architecture sim of intvec is
  signal v4d : std_logic_vector(3 downto 0) := "0000";
  signal v4a : std_logic_vector(0 to 3)     := "0000";
  signal b4  : bit_vector(3 downto 0)       := "0000";
  signal v8  : std_logic_vector(7 downto 0) := "00000000";
begin
end architecture;
