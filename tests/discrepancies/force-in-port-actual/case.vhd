-- LRM 10.5.2.1, 14.7.3.3: `force in` on a port of mode `in` sets the formal's effective value;
-- does the actual (whose effective value the formal normally takes) change too?
library ieee; use ieee.std_logic_1164.all;
entity sub is port (i : in std_logic_vector(3 downto 0); j : in integer); end entity;
architecture a of sub is
begin
  process begin
    wait for 1 ns;
    i <= force "1111";
    j <= force 9;
    wait for 1 ns;
    report "RES inside i=" & to_string(i) & " j=" & integer'image(j);
    wait;
  end process;
end architecture;
library ieee; use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  signal si : std_logic_vector(3 downto 0) := "0101";
  signal sj : integer := 1;
begin
  u : entity work.sub port map (i => si, j => sj);
  process begin
    wait for 1.5 ns;
    report "RES actual si=" & to_string(si) & " sj=" & integer'image(sj);
    wait;
  end process;
end architecture;
