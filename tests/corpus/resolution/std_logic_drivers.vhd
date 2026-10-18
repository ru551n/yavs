-- difftest: top=std_logic_drivers std=2008
library ieee; use ieee.std_logic_1164.all;
entity std_logic_drivers is end;
architecture t of std_logic_drivers is
  signal s : std_logic;
  signal v : std_logic_vector(1 downto 0);
begin
  s <= '1' when now < 10 ns else 'Z';
  s <= '0' when now >= 5 ns and now < 15 ns else 'Z';
  v <= "1Z"; v <= "Z0"; 
  process begin
    wait for 1 ns;
    report "s=" & std_logic'image(s) & " v=" & to_string(v);
    wait for 5 ns; report "s=" & std_logic'image(s);
    wait for 5 ns; report "s=" & std_logic'image(s);
    wait for 5 ns; report "s=" & std_logic'image(s);
    wait;
  end process;
end;
