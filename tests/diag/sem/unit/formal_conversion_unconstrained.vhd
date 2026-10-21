-- yavs-test: std=2008
-- expect: E1312 17:37 a conversion of formal `o` needs a fully constrained formal or conversion
-- LRM 6.5.7.1 (sim2 #2422; sim2 and sim3 reject; rule 1 review F8)
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity comp is
  port (o : out unsigned);
end entity;
architecture a of comp is
begin
  o <= (o'range => '0');
end architecture;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  signal sig : std_logic_vector(7 downto 0);
begin
  inst : entity work.comp port map (std_logic_vector(o) => sig);
end architecture;
