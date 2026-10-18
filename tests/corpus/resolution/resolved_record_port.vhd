-- difftest: top=resolved_record_port std=2008
library ieee; use ieee.std_logic_1164.all;
entity driver is port (o : out std_logic; v : in std_logic); end;
architecture a of driver is begin o <= v; end;
library ieee; use ieee.std_logic_1164.all;
entity resolved_record_port is end;
architecture t of resolved_record_port is
  signal bus_s : std_logic;
  signal x, y : std_logic := 'Z';
begin
  d1 : entity work.driver port map (bus_s, x);
  d2 : entity work.driver port map (bus_s, y);
  process begin
    wait for 1 ns; report std_logic'image(bus_s);
    x <= '1'; wait for 1 ns; report std_logic'image(bus_s);
    y <= '0'; wait for 1 ns; report std_logic'image(bus_s);
    x <= 'Z'; wait for 1 ns; report std_logic'image(bus_s);
    wait;
  end process;
end;
