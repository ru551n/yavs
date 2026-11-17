-- LRM 6.5.7.1, 14.7.3: a formal-side impure conversion reads a resolved signal that two ports drive and
-- that changes in the same cycle as the formal. New or old value of the signal?
library ieee;
use ieee.std_logic_1164.all;
entity drv5 is
  generic (v : std_logic_vector(7 downto 0));
  port (q : out std_logic_vector(7 downto 0));
end entity;
architecture a of drv5 is
begin
  q <= v, (others => 'Z') after 1 ns;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
entity src5 is
  port (o : out std_logic_vector(7 downto 0));
end entity;
architecture a of src5 is
begin
  o <= x"12", x"34" after 1 ns;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal ya, z : std_logic_vector(7 downto 0);
  impure function fa(p : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin return p xor ya; end function;
begin
  d1 : entity work.drv5 generic map (x"0F") port map (q => ya);
  d2 : entity work.drv5 generic map (x"0F") port map (q => ya);
  oa : entity work.src5 port map (fa(o) => z);
  process (z) begin report "RES z=" & to_hstring(z); end process;
end architecture;
