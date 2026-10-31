-- yavs-test: std=2008
-- expect: E1350 11:34 cannot convert
-- expect: E1301 12:34 does not fit the actual's type
-- LRM 9.3.6 and L72: the same checks as for a conversion of the whole formal (#617)
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity comp is port (o : out std_logic_vector(7 downto 0); io : inout std_logic_vector(7 downto 0)); end entity;
architecture a of comp is begin end architecture;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is signal n : integer; signal s : signed(7 downto 0); begin
  u : entity work.comp port map (integer(o(7 downto 0)) => n,
                                 signed(io(7 downto 0)) => s);
end architecture;
