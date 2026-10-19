-- LRM 6.5.7.1: a formal conversion on an unconstrained out port; the bounds come from the actual.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity sub is port (o : out unsigned); end entity;
architecture a of sub is begin o <= (o'range => '1'); end architecture;
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity top is end entity;
architecture a of top is
  signal s : std_logic_vector(3 downto 0);
begin
  u : entity work.sub port map (std_logic_vector(o) => s);
  process begin wait for 1 ns; report "RES " & to_string(s); wait; end process;
end architecture;
