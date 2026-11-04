-- A process stuck on a bus that keeps no history: the hang report gives the bus's last event.
library ieee; use ieee.std_logic_1164.all;
entity bare_stuck is end;
architecture a of bare_stuck is
  signal b, c : std_logic_vector(199 downto 0) := (others => '0');
begin
  c <= b;
  process begin
    wait on c(120 downto 60);
    wait until c(70) = 'X';
  end process;
  process begin
    b <= (others => '1');
    wait for 7 ns;
    b(3) <= '0';
    wait for 7 ns;
    b <= (70 => '1', others => '0');
    wait;
  end process;
end;
