-- yavs-test: std=2008
-- expect: E1313 12:34 a formal conversion needs a port of mode `out`, `inout` or `buffer`
-- LRM 6.5.7.1: a conversion of a part of a formal maps the formal's value out (#617)
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity comp is port (i : in std_logic_vector(7 downto 0)); end entity;
architecture a of comp is begin end architecture;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  signal x, y : signed(3 downto 0);
begin
  u : entity work.comp port map (signed(i(3 downto 0)) => x, i(7 downto 4) => std_logic_vector(y));
end architecture;
